//! 开发工具 bin：从 shared/common 的 50 张存量 entity 重建 PostgreSQL DDL。
//!
//! ```text
//! cargo run -p payment-service --bin gen_schema > body.sql
//! ```
//!
//! 输出仅为 50 张存量表的 `CREATE TABLE`（entity 可承载的部分）。
//! IAP 两表（iap_subscriptions / apple_iap_transactions）刻意不在生成范围——
//! entity 不承载 DEFAULT / CHECK / 索引 / RLS，以
//! `payment-service/schema_v0.1.sql` 的完整 DDL 为准，基准文件中内嵌那段。
//!
//! 为什么需要它：`genzltd_schema_v0.2.sql` 基准文件从未入库（仅 Cargo.toml
//! 注释引用），52 张存量表的 DDL 无处可查；entity 是唯一完整的数据模型真相。

use sea_orm::sea_query::TableCreateStatement;
use sea_orm::{DatabaseBackend, Schema};

fn legacy_tables() -> Vec<TableCreateStatement> {
    use common::entities::*;
    let s = Schema::new(DatabaseBackend::Postgres);
    vec![
        s.create_table_from_entity(act1_sessions::Entity),
        s.create_table_from_entity(applications::Entity),
        s.create_table_from_entity(audit_logs::Entity),
        s.create_table_from_entity(boost_consumptions::Entity),
        s.create_table_from_entity(boost_packs::Entity),
        s.create_table_from_entity(candidates::Entity),
        s.create_table_from_entity(challenge_entries::Entity),
        s.create_table_from_entity(challenge_rounds::Entity),
        s.create_table_from_entity(chunks::Entity),
        s.create_table_from_entity(consents::Entity),
        s.create_table_from_entity(documents::Entity),
        s.create_table_from_entity(domain_visits::Entity),
        s.create_table_from_entity(emergent_strategy_log::Entity),
        s.create_table_from_entity(enterprise_users::Entity),
        s.create_table_from_entity(enterprises::Entity),
        s.create_table_from_entity(fleet_members::Entity),
        s.create_table_from_entity(fleets::Entity),
        s.create_table_from_entity(game_profiles::Entity),
        s.create_table_from_entity(huawei_push_tokens::Entity),
        s.create_table_from_entity(invite_codes::Entity),
        s.create_table_from_entity(job_posts::Entity),
        s.create_table_from_entity(llm_usage::Entity),
        s.create_table_from_entity(match_consensus::Entity),
        s.create_table_from_entity(match_report_items::Entity),
        s.create_table_from_entity(match_reports::Entity),
        s.create_table_from_entity(micro_feedback::Entity),
        s.create_table_from_entity(mission_completions::Entity),
        s.create_table_from_entity(narrative_events::Entity),
        s.create_table_from_entity(narrative_feedback::Entity),
        s.create_table_from_entity(narrative_sessions::Entity),
        s.create_table_from_entity(navigator_memories::Entity),
        s.create_table_from_entity(orders::Entity),
        s.create_table_from_entity(poems::Entity),
        s.create_table_from_entity(product_events::Entity),
        s.create_table_from_entity(query_logs::Entity),
        s.create_table_from_entity(quiz_sessions::Entity),
        s.create_table_from_entity(quota_records::Entity),
        s.create_table_from_entity(report_sharing::Entity),
        s.create_table_from_entity(sales_inquiries::Entity),
        s.create_table_from_entity(share_codes::Entity),
        s.create_table_from_entity(signal_ingest::Entity),
        s.create_table_from_entity(subscriptions::Entity),
        s.create_table_from_entity(timeline_events::Entity),
        s.create_table_from_entity(trust_attestations::Entity),
        s.create_table_from_entity(trust_memories::Entity),
        s.create_table_from_entity(usage_logs::Entity),
        s.create_table_from_entity(user_identities::Entity),
        s.create_table_from_entity(users::Entity),
        s.create_table_from_entity(value_imprints::Entity),
        s.create_table_from_entity(verification_audit_log::Entity),
    ]
}

fn main() {
    let tables = legacy_tables();
    let total = tables.len();
    for (i, stmt) in tables.into_iter().enumerate() {
        let sql = stmt.to_string(sea_orm::sea_query::PostgresQueryBuilder);
        println!("-- [{i}/{total}]");
        println!("{sql};");
    }
}
