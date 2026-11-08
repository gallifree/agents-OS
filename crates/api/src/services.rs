use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Result;
use async_graphql::SimpleObject;
use fred::prelude::*;
use object_store::{ObjectStore, ObjectStoreExt, aws::AmazonS3Builder, path::Path};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::time::{Instant, timeout};

use crypto::Keys;

use crate::config::Config;

const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// Connections to every backing service. Cheap to clone.
#[derive(Clone)]
pub struct Services {
    pub cfg: Arc<Config>,
    pub pg: PgPool,
    pub valkey: Client,
    pub s3: Arc<dyn ObjectStore>,
    pub http: reqwest::Client,
}

impl Services {
    /// Connects lazily: the app starts even when a service is down, and the status query reports it.
    pub fn connect(cfg: Config) -> Result<Self> {
        // Fail fast on a bad MASTER_KEY. shortcut: keys aren't kept until sessions/IDs use them (milestone 2).
        Keys::from_master(&cfg.master_key)?;

        let pg = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(CHECK_TIMEOUT)
            .connect_lazy(&cfg.database_url)?;

        let valkey = Builder::from_config(fred::types::config::Config::from_url(&cfg.valkey_url)?)
            .set_policy(ReconnectPolicy::new_exponential(0, 100, 5_000, 2))
            .build()?;
        valkey.connect();

        let mut s3 = AmazonS3Builder::new()
            .with_region(&cfg.s3_region)
            .with_bucket_name(&cfg.s3_bucket)
            .with_access_key_id(&cfg.s3_access_key_id)
            .with_secret_access_key(&cfg.s3_secret_access_key);
        if let Some(endpoint) = &cfg.s3_endpoint {
            s3 = s3
                .with_endpoint(endpoint)
                .with_allow_http(endpoint.starts_with("http://"));
        }

        Ok(Self {
            cfg: Arc::new(cfg),
            pg,
            valkey,
            s3: Arc::new(s3.build()?),
            http: reqwest::Client::builder().timeout(CHECK_TIMEOUT).build()?,
        })
    }

    pub async fn status(&self) -> Vec<ServiceStatus> {
        let c = &self.cfg;
        let (postgres, valkey, typesense, storage, browser, docker, inference) = tokio::join!(
            check("postgres", false, async {
                let v: String = sqlx::query_scalar("SHOW server_version")
                    .fetch_one(&self.pg)
                    .await?;
                Ok(Some(v))
            }),
            check("valkey", false, async {
                let info: String = self
                    .valkey
                    .info(Some(fred::types::InfoKind::Server))
                    .await?;
                Ok(info_field(&info, "valkey_version")
                    .or_else(|| info_field(&info, "redis_version")))
            }),
            check("typesense", false, async {
                let debug = self
                    .get_json(
                        &format!("{}/debug", c.typesense_url),
                        &[("X-TYPESENSE-API-KEY", &c.typesense_api_key)],
                    )
                    .await?;
                Ok(str_field(&debug, "version"))
            }),
            check("storage", false, async {
                // A missing object proves the bucket answers with our credentials.
                match self.s3.head(&Path::from("__status__")).await {
                    Ok(_) | Err(object_store::Error::NotFound { .. }) => Ok(None),
                    Err(e) => Err(e.into()),
                }
            }),
            check("browser", false, async {
                // Chrome rejects a Host header that isn't an IP or localhost.
                let v = self
                    .get_json(
                        &format!("{}/json/version", c.browser_cdp_url),
                        &[("Host", "localhost")],
                    )
                    .await?;
                Ok(str_field(&v, "Browser"))
            }),
            check("docker", false, async {
                let v = self
                    .get_json(&format!("{}/version", c.docker_url), &[])
                    .await?;
                Ok(str_field(&v, "Version"))
            }),
            check("inference", true, async {
                let v = self
                    .get_json(&format!("{}/api/version", c.inference_url), &[])
                    .await?;
                Ok(str_field(&v, "version"))
            }),
        );
        vec![
            postgres, valkey, typesense, storage, browser, docker, inference,
        ]
    }

    async fn get_json(&self, url: &str, headers: &[(&str, &str)]) -> Result<Value> {
        let mut req = self.http.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        Ok(req.send().await?.error_for_status()?.json().await?)
    }
}

#[derive(SimpleObject, Debug)]
pub struct ServiceStatus {
    pub name: String,
    pub ok: bool,
    /// Optional services (e.g. inference running on the host) may be absent.
    pub optional: bool,
    pub version: Option<String>,
    pub latency_ms: u32,
    pub error: Option<String>,
}

async fn check(
    name: &str,
    optional: bool,
    probe: impl Future<Output = Result<Option<String>>>,
) -> ServiceStatus {
    let start = Instant::now();
    let result = timeout(CHECK_TIMEOUT, probe)
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")));
    let latency_ms = start.elapsed().as_millis() as u32;
    let (ok, version, error) = match result {
        Ok(version) => (true, version, None),
        Err(e) => (false, None, Some(format!("{e:#}"))),
    };
    ServiceStatus {
        name: name.into(),
        ok,
        optional,
        version,
        latency_ms,
        error,
    }
}

fn info_field(info: &str, key: &str) -> Option<String> {
    info.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .map(|v| v.trim().to_string())
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::info_field;

    #[test]
    fn reads_info_fields() {
        let info = "# Server\r\nredis_version:7.2.4\r\nvalkey_version:9.0.1\r\n";
        assert_eq!(info_field(info, "valkey_version").as_deref(), Some("9.0.1"));
        assert_eq!(info_field(info, "redis_version").as_deref(), Some("7.2.4"));
        assert_eq!(info_field(info, "missing"), None);
    }
}
