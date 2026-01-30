use config::{Config as ConfigBuilder, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub redis_url: String,
    pub jwt_secret: String,
    pub jwt_expiration: i64,
    pub consul_url: String,
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        // 加载.env文件
        dotenvy::dotenv().ok();

        let config = ConfigBuilder::builder()
            // 默认值
            .set_default("port", 3001)?
            .set_default(
                "database_url",
                "mysql://root:adirp_password@localhost/adirp_auth",
            )?
            .set_default("redis_url", "redis://localhost/")?
            .set_default("jwt_secret", "your-secret-key-please-change-in-production")?
            .set_default("jwt_expiration", 86400)? // 24小时
            .set_default("consul_url", "http://localhost:8500")?
            // 从环境变量加载（覆盖默认值）
            .add_source(Environment::default())
            .build()?;

        config.try_deserialize()
    }
}
