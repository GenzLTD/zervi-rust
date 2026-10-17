//! 记忆层门面：把 `mcp-client` 的四个方法包成「**失败可降级**」的形态。
//!
//! ## 为什么要有这一层
//!
//! `mcp-client` 只回答「MCP 调用成不成功」。而 `upload-service` 需要回答的是
//! 另一个问题：**这次 `/upload` 到底用没用上记忆、没用上的原因是什么**。
//!
//! 终端机是单用户、单机，用户**看不到服务端日志**，所以降级必须进响应体
//! （`memory_used` / `memory_skip_reason`），否则就是静默功能退化 —— 比 500
//! 更难查。本模块把「不可用」统一收敛成一个 `Err(原因)`，由 handler 写进响应。
//!
//! ## 失败语义（已拍定）
//!
//! **跳过记忆继续**，不是失败退出。终端机的主链路是「优化简历」，记忆是增强项。

use std::sync::Arc;
use std::time::Duration;

use mcp_client::{Lifecycle, McpMemoryClient, McpServerSpec, MemoryHit};

use crate::config::Config;

/// 记忆层门面。`inner == None` 时所有调用都返回 `Err(disabled_reason)`。
pub struct MemoryLayer {
    inner: Option<Arc<McpMemoryClient>>,
    /// 未启用 / 启动失败的原因，原样作为 `memory_skip_reason`。
    disabled_reason: Option<String>,
}

impl MemoryLayer {
    /// 显式禁用（`MCP_ENABLED=false` 或测试用）。
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self {
            inner: None,
            disabled_reason: Some(reason.into()),
        }
    }

    /// 按配置拉起 cortex-mcp 子进程。
    ///
    /// 关键点：**DSN 注入**。cortex-mcp 的 `config.py` 默认 DSN 是
    /// `postgresql://localhost:5432/cortex`，而 compose 把宿主 **5433** 映射进容器。
    /// 不注入就会连错端口 —— 握手不碰库所以看不出来，一落库才炸。
    /// 所以这里把 `CORTEX_DATABASE_URL` 显式塞进子进程环境（env 前缀是 `CORTEX_`）。
    pub async fn from_config(config: &Config) -> Self {
        if !config.mcp_enabled {
            return Self::disabled("MCP_ENABLED=false（记忆层未启用）");
        }

        let mut spec = McpServerSpec::python_module(
            config.mcp_python.clone(),
            config.mcp_module.clone(),
        );
        if let Some(cwd) = &config.mcp_cwd {
            spec = spec.with_cwd(cwd.clone());
        }
        if let Some(dsn) = &config.cortex_database_url {
            spec = spec.with_env("CORTEX_DATABASE_URL", dsn.clone());
        }

        let connect_timeout = Duration::from_millis(config.mcp_connect_timeout_ms);
        // 生产固定 Legacy：Mac 真机 + Dell 夹具对拍一致确认 Python `mcp` 1.x 只认 initialize。
        let attempt = tokio::time::timeout(
            connect_timeout,
            McpMemoryClient::connect(&spec, Lifecycle::Legacy),
        )
        .await;

        match attempt {
            Ok(Ok(client)) => {
                let client = client.with_timeouts(
                    Duration::from_millis(config.mcp_recall_timeout_ms),
                    Duration::from_millis(config.mcp_write_timeout_ms),
                );
                tracing::info!(
                    "🧠 记忆层已连接（cortex-mcp, protocolVersion={}, recall 上限 {}ms, 写上限 {}ms）",
                    client.protocol_version().unwrap_or_else(|| "?".into()),
                    config.mcp_recall_timeout_ms,
                    config.mcp_write_timeout_ms
                );
                Self {
                    inner: Some(Arc::new(client)),
                    disabled_reason: None,
                }
            }
            Ok(Err(e)) => {
                let reason = format!("记忆层启动失败: {e:#}");
                tracing::warn!("⚠️  {} —— /upload 将跳过记忆继续", reason);
                Self::disabled(reason)
            }
            Err(_) => {
                let reason = format!(
                    "记忆层启动超时（>{}ms）",
                    config.mcp_connect_timeout_ms
                );
                tracing::warn!("⚠️  {} —— /upload 将跳过记忆继续", reason);
                Self::disabled(reason)
            }
        }
    }

    /// 是否真的可用（不代表下一次调用一定成功，但代表进程已握手）。
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// 检索记忆。`Err` = 跳过原因（未启用 / 超时 / 调用失败 / 子进程已死）。
    pub async fn recall(
        &self,
        query: &str,
        kind: Option<&str>,
        k: usize,
    ) -> Result<Vec<MemoryHit>, String> {
        match &self.inner {
            None => Err(self
                .disabled_reason
                .clone()
                .unwrap_or_else(|| "记忆层不可用".to_string())),
            Some(client) => {
                if !client.is_alive() {
                    return Err("记忆层子进程已退出（重启策略未实现）".to_string());
                }
                client.recall(query, kind, k).await
            }
        }
    }

    /// 写入一条记忆。`Err` = 跳过原因。
    pub async fn remember(
        &self,
        content: &str,
        kind: Option<&str>,
        metadata: Option<serde_json::Value>,
        importance: Option<f64>,
    ) -> Result<String, String> {
        match &self.inner {
            None => Err(self
                .disabled_reason
                .clone()
                .unwrap_or_else(|| "记忆层不可用".to_string())),
            Some(client) => {
                if !client.is_alive() {
                    return Err("记忆层子进程已退出（重启策略未实现）".to_string());
                }
                client.remember(content, kind, metadata, importance).await
            }
        }
    }

    /// 关闭子进程（进程退出时调用）。
    pub async fn close(&self) {
        if let Some(client) = &self.inner {
            client.close().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 未启用时：recall / remember 都必须返回**可读原因**，而不是 panic 或 timeout。
    /// 这是「跳过记忆继续」这条语义的最小保证 —— handler 直接把它写进响应体。
    #[tokio::test]
    async fn disabled_layer_yields_reason() {
        let layer = MemoryLayer::disabled("MCP_ENABLED=false（测试）");
        assert!(!layer.is_enabled());

        let err = layer.recall("任意查询", None, 5).await.unwrap_err();
        assert!(err.contains("MCP_ENABLED=false"), "实际原因: {err}");

        let err = layer.remember("内容", Some("episodic"), None, None).await.unwrap_err();
        assert!(err.contains("MCP_ENABLED=false"), "实际原因: {err}");
    }

    /// 启用但**对端拉不起来**时：构造阶段就降级（不 panic、不无限等），
    /// 且原因里能看出是「启动失败」。这条覆盖 `from_config` 的失败分支。
    #[tokio::test]
    async fn unreachable_server_degrades_at_startup() {
        // 只影响本进程测试环境；Config::from_env 在调用时才读取。
        std::env::set_var("MCP_ENABLED", "true");
        std::env::set_var("MCP_PYTHON", "C:/definitely/not/here/python.exe");
        std::env::set_var("MCP_MODULE", "cortex_mcp.server");
        std::env::set_var("MCP_CONNECT_TIMEOUT_MS", "3000");

        let config = crate::config::Config::from_env();
        let started = std::time::Instant::now();
        let layer = MemoryLayer::from_config(&config).await;

        std::env::remove_var("MCP_ENABLED");
        std::env::remove_var("MCP_PYTHON");
        std::env::remove_var("MCP_MODULE");
        std::env::remove_var("MCP_CONNECT_TIMEOUT_MS");

        assert!(!layer.is_enabled());
        let err = layer.recall("q", None, 5).await.unwrap_err();
        assert!(err.contains("记忆层启动失败"), "实际原因: {err}");
        // 子进程都拉不起来时必须**快速失败**，不能卡满 connect timeout
        assert!(started.elapsed().as_secs() < 3, "耗时 {:?}", started.elapsed());
    }
}
