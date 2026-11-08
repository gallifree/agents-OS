use anyhow::{Context, Result};

/// Runtime configuration, read once from the environment (see deploy/.env.example).
/// No `Debug`: it holds secrets that must never reach logs.
#[derive(Clone)]
pub struct Config {
    pub bind_addr: String,
    pub master_key: String,
    pub web_dir: String,
    pub database_url: String,
    pub valkey_url: String,
    pub typesense_url: String,
    pub typesense_api_key: String,
    pub s3_endpoint: Option<String>,
    pub s3_region: String,
    pub s3_bucket: String,
    pub s3_access_key_id: String,
    pub s3_secret_access_key: String,
    pub browser_cdp_url: String,
    pub docker_url: String,
    pub inference_url: String,
    /// Adds `Secure` to the session cookie. Turn on whenever the app is served over HTTPS.
    pub cookie_secure: bool,
}

fn var(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("missing environment variable {name}"))
}

fn var_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.into())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            bind_addr: var_or("BIND_ADDR", "0.0.0.0:8080"),
            master_key: var("MASTER_KEY")?,
            web_dir: var_or("WEB_DIR", "web/dist/client"),
            database_url: var("DATABASE_URL")?,
            valkey_url: var_or("VALKEY_URL", "redis://valkey:6379"),
            typesense_url: var_or("TYPESENSE_URL", "http://typesense:8108"),
            typesense_api_key: var("TYPESENSE_API_KEY")?,
            s3_endpoint: std::env::var("S3_ENDPOINT").ok().filter(|s| !s.is_empty()),
            s3_region: var_or("S3_REGION", "garage"),
            s3_bucket: var_or("S3_BUCKET", "assets"),
            s3_access_key_id: var("S3_ACCESS_KEY_ID")?,
            s3_secret_access_key: var("S3_SECRET_ACCESS_KEY")?,
            browser_cdp_url: var_or("BROWSER_CDP_URL", "http://browser:9222"),
            // Docker clients use tcp://; we talk to the socket proxy over plain HTTP.
            docker_url: var_or("DOCKER_HOST", "tcp://docker-proxy:2375")
                .replacen("tcp://", "http://", 1),
            inference_url: var_or("INFERENCE_URL", "http://inference:11434"),
            cookie_secure: var_or("COOKIE_SECURE", "false") == "true",
        })
    }
}
