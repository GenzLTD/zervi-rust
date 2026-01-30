// 真实的Consul客户端实现
// 使用Consul HTTP API

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time;

/// Consul服务注册
#[derive(Debug, Serialize)]
pub struct ServiceRegistration {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Tags")]
    pub tags: Vec<String>,
    #[serde(rename = "Address")]
    pub address: String,
    #[serde(rename = "Port")]
    pub port: u16,
    #[serde(rename = "Check")]
    pub check: ServiceCheck,
}

/// Consul健康检查配置
#[derive(Debug, Serialize)]
pub struct ServiceCheck {
    #[serde(rename = "HTTP")]
    pub http: String,
    #[serde(rename = "Interval")]
    pub interval: String,
    #[serde(rename = "Timeout")]
    pub timeout: String,
    #[serde(rename = "DeregisterCriticalServiceAfter")]
    pub deregister_critical_service_after: String,
}

/// Consul服务实例
#[derive(Debug, Deserialize)]
pub struct ServiceInstance {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Service")]
    pub service: String,
    #[serde(rename = "Tags")]
    pub tags: Vec<String>,
    #[serde(rename = "Address")]
    pub address: String,
    #[serde(rename = "Port")]
    pub port: u16,
    #[serde(rename = "Meta")]
    pub meta: Option<std::collections::HashMap<String, String>>,
}

/// 服务注册客户端
pub struct ServiceRegistry {
    consul_url: String,
    client: reqwest::Client,
    service_id: String,
    service_name: String,
    service_port: u16,
    service_address: String,
}

impl ServiceRegistry {
    /// 创建新的服务注册实例
    pub fn new(service_name: &str, port: u16) -> Self {
        Self::with_address(service_name, port, "127.0.0.1")
    }

    /// 创建新的服务注册实例（指定地址）
    pub fn with_address(service_name: &str, port: u16, address: &str) -> Self {
        let consul_url =
            std::env::var("CONSUL_URL").unwrap_or_else(|_| "http://localhost:8500".to_string());

        Self {
            consul_url,
            client: reqwest::Client::new(),
            service_id: format!("{}-{}", service_name, port),
            service_name: service_name.to_string(),
            service_port: port,
            service_address: address.to_string(),
        }
    }

    /// 注册服务到Consul
    pub async fn register(&self) -> Result<()> {
        let health_check_url = format!(
            "http://{}:{}/health",
            self.service_address, self.service_port
        );

        let registration = ServiceRegistration {
            id: self.service_id.clone(),
            name: self.service_name.clone(),
            tags: vec!["rust".to_string(), "adirp".to_string()],
            address: self.service_address.clone(),
            port: self.service_port,
            check: ServiceCheck {
                http: health_check_url,
                interval: "10s".to_string(),
                timeout: "5s".to_string(),
                deregister_critical_service_after: "30s".to_string(),
            },
        };

        let url = format!("{}/v1/agent/service/register", self.consul_url);

        let response = self
            .client
            .put(&url)
            .json(&registration)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to register service: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Consul registration failed with status {}: {}",
                status,
                body
            ));
        }

        tracing::info!(
            "✅ Service {} registered to Consul at {}:{}",
            self.service_name,
            self.service_address,
            self.service_port
        );

        Ok(())
    }

    /// 注销服务
    pub async fn deregister(&self) -> Result<()> {
        let url = format!(
            "{}/v1/agent/service/deregister/{}",
            self.consul_url, self.service_id
        );

        let response = self
            .client
            .put(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to deregister service: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Consul deregistration failed with status {}: {}",
                status,
                body
            ));
        }

        tracing::info!("Service {} deregistered from Consul", self.service_name);
        Ok(())
    }

    /// 启动心跳任务（通过健康检查自动维护）
    pub async fn start_heartbeat(&self) -> Result<()> {
        let service_name = self.service_name.clone();

        // Consul的HTTP健康检查会自动维护服务状态
        // 我们只需要确保 /health 端点正常工作即可
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                tracing::trace!(
                    "❤️  Service {} is being health checked by Consul",
                    service_name
                );
            }
        });

        tracing::info!(
            "❤️  Health check endpoint configured for {}",
            self.service_name
        );
        Ok(())
    }
}

/// 服务发现客户端
pub struct ServiceDiscovery {
    consul_url: String,
    client: reqwest::Client,
}

impl ServiceDiscovery {
    pub fn new() -> Self {
        let consul_url =
            std::env::var("CONSUL_URL").unwrap_or_else(|_| "http://localhost:8500".to_string());

        Self {
            consul_url,
            client: reqwest::Client::new(),
        }
    }

    /// 发现服务（获取所有实例）
    pub async fn discover_service(&self, service_name: &str) -> Result<Option<String>> {
        let instances = self.get_service_instances(service_name).await?;

        if let Some(instance) = instances.first() {
            let url = format!("http://{}:{}", instance.address, instance.port);
            tracing::debug!("🔍 Discovered service {} at {}", service_name, url);
            Ok(Some(url))
        } else {
            tracing::warn!("Service {} not found in Consul", service_name);
            Ok(None)
        }
    }

    /// 获取服务的所有实例
    async fn get_service_instances(&self, service_name: &str) -> Result<Vec<ServiceInstance>> {
        let url = format!("{}/v1/catalog/service/{}", self.consul_url, service_name);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to query Consul: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Consul query failed with status: {}",
                response.status()
            ));
        }

        let instances: Vec<ServiceInstance> = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Consul response: {}", e))?;

        Ok(instances)
    }

    /// 获取所有健康的服务实例
    pub async fn get_healthy_services(&self, service_name: &str) -> Result<Vec<String>> {
        let url = format!(
            "{}/v1/health/service/{}?passing=true",
            self.consul_url, service_name
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to query Consul health: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Consul health query failed with status: {}",
                response.status()
            ));
        }

        #[derive(Deserialize)]
        struct HealthCheck {
            #[serde(rename = "Service")]
            service: ServiceInstance,
        }

        let health_checks: Vec<HealthCheck> = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Consul health response: {}", e))?;

        let urls: Vec<String> = health_checks
            .iter()
            .map(|h| format!("http://{}:{}", h.service.address, h.service.port))
            .collect();

        tracing::debug!("Found {} healthy instances of {}", urls.len(), service_name);

        Ok(urls)
    }

    /// 获取所有已注册的服务名称
    pub async fn list_services(&self) -> Result<Vec<String>> {
        let url = format!("{}/v1/catalog/services", self.consul_url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to list services: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Consul list services failed with status: {}",
                response.status()
            ));
        }

        let services: std::collections::HashMap<String, Vec<String>> = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse services list: {}", e))?;

        Ok(services.keys().cloned().collect())
    }
}

impl Default for ServiceDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // 需要Consul运行
    async fn test_service_registration() {
        let registry = ServiceRegistry::new("test-service", 9999);
        assert!(registry.register().await.is_ok());
        assert!(registry.deregister().await.is_ok());
    }

    #[tokio::test]
    #[ignore] // 需要Consul运行
    async fn test_service_discovery() {
        let discovery = ServiceDiscovery::new();
        let services = discovery.list_services().await.unwrap();
        println!("Found services: {:?}", services);
    }
}
