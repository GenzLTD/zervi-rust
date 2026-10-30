//! 过期 grant 的后台清扫 —— 授权面的第三道口子（契约里明确划归后台）。
//!
//! `resume.v1.json` → `permission_semantics.grants.expires_at_semantics`：
//! 过期条目读响应不返回、业务判定按无授权处理，端点只做「读到就清」（GET）
//! 与「写前剔除」（PUT），**全表清扫归后台任务，不在端点语义内**。问题是没人读的
//! 文档永远不会被 GET 碰到，过期条目就长期脏积在 `metadata` 里（本模块补这一面）。
//!
//! 三条一致性要求，所以不重写判定、直接复用 routes 的实现：
//! · 过期判定 `partition_grants`：RFC3339 解析失败 = 未过期，与读路径同口径；
//! · 写回形状：`grants = live` + `updated_at` 重置为清扫时刻（契约 write_semantics）；
//! · 写入路径 `persist_document`：正文与授权面共用同一条写回，不另起第二条 UPDATE。
//!
//! 零过期零写放大：SQL 只做**文本正则粗筛**（`metadata` 是 text 列，不 cast 成 jsonb，
//! 脏值不可能炸查询），过期判定全在 Rust；粗筛命中但 `expired.is_empty()` 的行直接
//! 跳过、不产生 UPDATE——包括「刚被 GET 惰性清掉」的并发行。
//!
//! ⚠️ 脏行连带删除：写回按 `live` 落库，因此**同一文档里解析失败的脏 grant 会被这次
//! 写回一起删掉**（它本来也不出现在任何 API 响应里）。纯脏行文档（唯一 grant 的
//! expires_at 解析失败）粗筛正则就过不了，压根不进扫描集，脏值原样保留。
//!
//! 局限（挂账，不在本刀）：每次 tick 是 `documents.metadata` 的 regex 全表扫，表大了
//! 要加 expression index；多副本各自扫一遍（写回幂等，只是浪费），真上多副本再抽
//! 独立 worker / 单实例 leader 选举。

use common::entities::documents;
use common::looma::{now_tz, AppState};
use sea_orm::{ConnectionTrait, EntityTrait};
use serde_json::json;
use std::time::Duration;

use crate::routes::{parse_meta, partition_grants, persist_document};

/// 默认 15 分钟一轮。默认必须是**开**的——挂账的病根就是「没人读就脏积」，
/// 给个默认关的开关等于没修。
const DEFAULT_INTERVAL_SECS: u64 = 900;

/// 单轮候选上限。够覆盖当前量级；超出的留给下一轮（顺序按 id，不会饿死）。
const BATCH: i64 = 500;

/// 粗筛：只要 metadata 里出现「permission + 带日期前缀的 expires_at」，就可能有过期条目。
/// 刻意不 cast jsonb、不判 `<= now()`：判定只留一处（Rust `partition_grants`），
/// 否则 SQL 与端点各一套 RFC3339 口径，迟早对不上。
const CANDIDATE_SQL: &str = r#"
SELECT id
FROM documents
WHERE metadata ~ '"permission"'
  AND metadata ~ '"expires_at"[[:space:]]*:[[:space:]]*"[0-9]{4}-[0-9]{2}-[0-9]{2}T'
ORDER BY id
LIMIT $1
"#;

/// 一轮清扫的结果，仅用于日志与复烟取证。
#[derive(Debug, Default)]
pub struct SweepStats {
    /// 粗筛命中并逐行复核的文档数
    pub scanned: u64,
    /// 真被改写的文档数（每条 UPDATE 计一次）
    pub cleared: u64,
    /// 被剔除的过期 grant 条目数
    pub expired: u64,
}

/// 起后台清扫。`RESUME_SWEEP_INTERVAL_SECS=0` 关闭（本机排障用）；
/// 非数字回落到默认值，**不静默关掉**——拒绝把配置写错翻译成「清理停摆」。
pub fn spawn(state: AppState) {
    let raw = std::env::var("RESUME_SWEEP_INTERVAL_SECS").ok();
    let secs = raw
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s.parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    value = %s,
                    "RESUME_SWEEP_INTERVAL_SECS 不是整数，回落默认间隔"
                );
                DEFAULT_INTERVAL_SECS
            }
        })
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    if secs == 0 {
        tracing::info!("grants sweep 已关闭（RESUME_SWEEP_INTERVAL_SECS=0）");
        return;
    }

    tokio::spawn(async move {
        // 启动先跑一轮：等服务第一个 tick 才清，等于把重启前的脏数据又留一个周期。
        run_once(&state).await;

        let mut ticker = tokio::time::interval(Duration::from_secs(secs));
        // 一轮扫得比间隔还久时，默认 Burst 会把错过的 tick 连续补跑（堆积）。
        // 清扫是幂等的，丢掉错过的 tick 比连跑几遍好。
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // interval 的首次 tick 立即完成；上面已手跑过一轮，先消费掉这个空 tick。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            run_once(&state).await;
        }
    });
    tracing::info!(interval_secs = secs, "grants sweep 已启动");
}

async fn run_once(state: &AppState) {
    match sweep_once(state).await {
        Ok(stats) => {
            // 复烟按这行取数；swept/cleared 分开才能证明「零过期不写库」。
            tracing::info!(
                scanned = stats.scanned,
                cleared = stats.cleared,
                expired = stats.expired,
                "grants sweep 完成"
            );
        }
        Err(e) => tracing::warn!(error = %e, "grants sweep 失败，等下一轮"),
    }
}

/// 单轮清扫。返回 `Err` 只表示候选查询失败（连库问题），单行失败只记日志不中断整轮。
pub async fn sweep_once(state: &AppState) -> Result<SweepStats, sea_orm::DbErr> {
    let now = now_tz();
    let ids = candidate_ids(state).await?;
    let mut stats = SweepStats::default();

    for id in ids {
        // 命中 id 后再取整行：persist_document 按整行回写其余列，
        // 用粗筛时的快照会覆盖掉并发改动（正文/title），所以这里读最新值。
        let row = match documents::Entity::find_by_id(id).one(&state.db).await {
            Ok(Some(row)) => row,
            Ok(None) => continue, // 粗筛与读之间被删
            Err(e) => {
                tracing::warn!(document_id = id, error = %e, "sweep 读行失败，跳过");
                continue;
            }
        };
        stats.scanned += 1;

        let mut meta = parse_meta(row.metadata.as_deref());
        let (live, expired) = partition_grants(&meta, now);
        if expired.is_empty() {
            continue; // 零过期零写放大
        }

        // expired 非空 ⟹ permission.grants 存在且是数组 ⟹ 父级 permission 一定在，
        // 下面的下标赋值不会把 null 当对象去写。
        let expired_count = expired.len() as u64;
        meta["permission"]["grants"] = json!(live);
        meta["permission"]["updated_at"] = json!(now.to_rfc3339());
        match persist_document(state, row, meta, None, None).await {
            Ok(_) => {
                stats.cleared += 1;
                stats.expired += expired_count;
            }
            Err(resp) => {
                tracing::warn!(
                    document_id = id,
                    status = resp.status().as_u16(),
                    "sweep 回写失败，跳过"
                );
            }
        }
    }

    Ok(stats)
}

async fn candidate_ids(state: &AppState) -> Result<Vec<i64>, sea_orm::DbErr> {
    let stmt = sea_orm::Statement::from_sql_and_values(
        state.db.get_database_backend(),
        CANDIDATE_SQL,
        [(BATCH).into()],
    );
    let rows = state.db.query_all(stmt).await?;
    Ok(rows
        .iter()
        .filter_map(|r| r.try_get::<i64>("", "id").ok())
        .collect())
}
