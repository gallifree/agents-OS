//! LLM gateway administration: providers (admins), API keys (each user their own) and usage.

use async_graphql::{Context, ID, Object, Result, SimpleObject};
use chrono::{DateTime, Utc};
use crypto::Keys;
use fred::prelude::*;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::{ID_TTL, coded, decode_typed, internal, require_admin, require_viewer, services};
use crate::{
    gateway::{self, MODELS_CACHE_KEY, Upstream},
    services::Services,
};

/// An OpenAI-compatible upstream. Its models are reached as `"<name>/<model>"` on `/v1`.
#[derive(SimpleObject, Clone)]
pub struct Provider {
    pub id: ID,
    pub cache_key: String,
    pub name: String,
    pub base_url: String,
    /// The key itself is never sent back.
    pub has_api_key: bool,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct ProviderRow {
    id: Uuid,
    name: String,
    base_url: String,
    has_api_key: bool,
    enabled: bool,
    created_at: DateTime<Utc>,
}

impl Provider {
    fn from_row(row: ProviderRow, keys: &Keys) -> Self {
        Self {
            id: ID(keys.encode_id("Provider", row.id, ID_TTL)),
            cache_key: keys.cache_key("Provider", row.id),
            name: row.name,
            base_url: row.base_url,
            has_api_key: row.has_api_key,
            enabled: row.enabled,
            created_at: row.created_at,
        }
    }
}

macro_rules! provider_columns {
    () => {
        "id, name, base_url, api_key IS NOT NULL AS has_api_key, enabled, created_at"
    };
}

pub async fn load_provider(pg: &PgPool, keys: &Keys, id: Uuid) -> Result<Option<Provider>> {
    let row: Option<ProviderRow> = sqlx::query_as(concat!(
        "SELECT ",
        provider_columns!(),
        " FROM llm_providers WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pg)
    .await
    .map_err(internal)?;
    Ok(row.map(|r| Provider::from_row(r, keys)))
}

/// A bearer token for `/v1`. Only its hash is stored.
#[derive(SimpleObject, Clone)]
pub struct ApiKey {
    pub id: ID,
    pub cache_key: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(FromRow)]
struct ApiKeyRow {
    id: Uuid,
    name: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

impl ApiKey {
    fn from_row(row: ApiKeyRow, keys: &Keys) -> Self {
        Self {
            id: ID(keys.encode_id("ApiKey", row.id, ID_TTL)),
            cache_key: keys.cache_key("ApiKey", row.id),
            name: row.name,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

/// Only the owner's own, unrevoked keys.
pub async fn load_api_key(
    pg: &PgPool,
    keys: &Keys,
    owner: Uuid,
    id: Uuid,
) -> Result<Option<ApiKey>> {
    let row: Option<ApiKeyRow> = sqlx::query_as(
        "SELECT id, name, created_at, last_used_at FROM api_keys
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(owner)
    .fetch_optional(pg)
    .await
    .map_err(internal)?;
    Ok(row.map(|r| ApiKey::from_row(r, keys)))
}

#[derive(SimpleObject)]
pub struct CreatedApiKey {
    pub api_key: ApiKey,
    /// The token, returned this once only.
    pub token: String,
}

/// Requests per provider and model over a period.
#[derive(SimpleObject, FromRow)]
pub struct ModelUsage {
    pub provider: String,
    pub model: String,
    pub requests: i64,
    /// Requests answered with an error status (≥ 400, including 499: client gone).
    pub errors: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

// ---- Validation -----------------------------------------------------------------------------

fn validate_name(name: &str) -> Result<()> {
    let ok = (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && !name.starts_with(['-', '_']);
    ok.then_some(()).ok_or_else(|| {
        coded(
            "INVALID_PROVIDER_NAME",
            "1–32 characters: a-z, 0-9, '-' or '_'",
        )
    })
}

/// `https://api.openai.com/v1`, `http://inference:11434/v1`… (admins choose it, so internal hosts are allowed).
fn validate_url(url: &str) -> Result<String> {
    let parsed =
        reqwest::Url::parse(url.trim()).map_err(|_| coded("INVALID_URL", "invalid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err(coded("INVALID_URL", "the URL must be http(s)://host/…"));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

/// Empty means "no key" (local providers such as Ollama).
fn sealed_key(keys: &Keys, id: Uuid, api_key: &str) -> Option<Vec<u8>> {
    let key = api_key.trim();
    (!key.is_empty()).then(|| keys.encrypt_secret(&gateway::secret_context(id), key.as_bytes()))
}

async fn forget_models(s: &Services) {
    if let Err(e) = s.valkey.del::<i64, _>(MODELS_CACHE_KEY).await {
        tracing::warn!("clearing the llm models cache: {e}");
    }
}

// ---- Resolvers ------------------------------------------------------------------------------

#[derive(Default)]
pub struct LlmQuery;

#[Object]
impl LlmQuery {
    /// Admins only.
    async fn providers(&self, ctx: &Context<'_>) -> Result<Vec<Provider>> {
        require_admin(ctx)?;
        let s = services(ctx);
        let rows: Vec<ProviderRow> = sqlx::query_as(concat!(
            "SELECT ",
            provider_columns!(),
            " FROM llm_providers ORDER BY name"
        ))
        .fetch_all(&s.pg)
        .await
        .map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|r| Provider::from_row(r, &s.keys))
            .collect())
    }

    /// Asks the provider for its models: also the connection test. Admins only.
    async fn provider_models(&self, ctx: &Context<'_>, id: ID) -> Result<Vec<String>> {
        require_admin(ctx)?;
        let s = services(ctx);
        let uuid = decode_typed(&s.keys, &id, "Provider")?;
        let row: Option<(String, String, Option<Vec<u8>>)> =
            sqlx::query_as("SELECT name, base_url, api_key FROM llm_providers WHERE id = $1")
                .bind(uuid)
                .fetch_optional(&s.pg)
                .await
                .map_err(internal)?;
        let (name, base_url, sealed) = row.ok_or_else(|| coded("NOT_FOUND", "no such provider"))?;
        let api_key = sealed
            .map(|c| s.keys.decrypt_secret(&gateway::secret_context(uuid), &c))
            .transpose()
            .map_err(|_| internal("provider key does not decrypt (master key changed?)"))?
            .map(|k| String::from_utf8_lossy(&k).into_owned());
        Upstream::new(uuid, name, base_url, api_key)
            .models(s)
            .await
            .map_err(|e| coded("PROVIDER_UNREACHABLE", format!("{e:#}")))
    }

    /// The viewer's own API keys.
    async fn api_keys(&self, ctx: &Context<'_>) -> Result<Vec<ApiKey>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let rows: Vec<ApiKeyRow> = sqlx::query_as(
            "SELECT id, name, created_at, last_used_at FROM api_keys
             WHERE user_id = $1 AND revoked_at IS NULL ORDER BY id",
        )
        .bind(viewer.user_id)
        .fetch_all(&s.pg)
        .await
        .map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|r| ApiKey::from_row(r, &s.keys))
            .collect())
    }

    /// Usage over the last `days` days (1–365): everyone's for admins, the viewer's own otherwise.
    async fn llm_usage(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 30)] days: i32,
    ) -> Result<Vec<ModelUsage>> {
        let viewer = require_viewer(ctx)?;
        let only = (!viewer.is_admin).then_some(viewer.user_id);
        sqlx::query_as(
            "SELECT provider, model, count(*) AS requests,
                    count(*) FILTER (WHERE status >= 400) AS errors,
                    coalesce(sum(prompt_tokens), 0)::bigint AS prompt_tokens,
                    coalesce(sum(completion_tokens), 0)::bigint AS completion_tokens
             FROM llm_usage
             WHERE created_at > now() - make_interval(days => $1) AND ($2::uuid IS NULL OR user_id = $2)
             GROUP BY provider, model ORDER BY requests DESC, provider, model",
        )
        .bind(days.clamp(1, 365))
        .bind(only)
        .fetch_all(&services(ctx).pg)
        .await
        .map_err(internal)
    }
}

#[derive(Default)]
pub struct LlmMutation;

#[Object]
impl LlmMutation {
    /// Admins only. `apiKey` may be empty for local providers.
    async fn create_provider(
        &self,
        ctx: &Context<'_>,
        name: String,
        base_url: String,
        #[graphql(default)] api_key: String,
    ) -> Result<Provider> {
        require_admin(ctx)?;
        let s = services(ctx);
        validate_name(&name)?;
        let base_url = validate_url(&base_url)?;
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO llm_providers (id, name, base_url, api_key) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(&name)
        .bind(base_url)
        .bind(sealed_key(&s.keys, id, &api_key))
        .execute(&s.pg)
        .await
        .map_err(|e| match e.as_database_error() {
            Some(d) if d.is_unique_violation() => coded("PROVIDER_EXISTS", "name already used"),
            _ => internal(e),
        })?;
        forget_models(s).await;
        load_provider(&s.pg, &s.keys, id)
            .await?
            .ok_or_else(|| internal("provider vanished"))
    }

    /// Admins only. Omitted fields keep their value; an empty `apiKey` removes the key.
    async fn update_provider(
        &self,
        ctx: &Context<'_>,
        id: ID,
        base_url: Option<String>,
        api_key: Option<String>,
        enabled: Option<bool>,
    ) -> Result<Provider> {
        require_admin(ctx)?;
        let s = services(ctx);
        let uuid = decode_typed(&s.keys, &id, "Provider")?;
        let base_url = base_url.map(|u| validate_url(&u)).transpose()?;
        let sealed = api_key.as_deref().map(|k| sealed_key(&s.keys, uuid, k));
        let updated = sqlx::query(
            "UPDATE llm_providers SET
                base_url = coalesce($2, base_url),
                api_key = CASE WHEN $3 THEN $4 ELSE api_key END,
                enabled = coalesce($5, enabled)
             WHERE id = $1",
        )
        .bind(uuid)
        .bind(base_url)
        .bind(sealed.is_some())
        .bind(sealed.flatten())
        .bind(enabled)
        .execute(&s.pg)
        .await
        .map_err(internal)?;
        if updated.rows_affected() == 0 {
            return Err(coded("NOT_FOUND", "no such provider"));
        }
        forget_models(s).await;
        load_provider(&s.pg, &s.keys, uuid)
            .await?
            .ok_or_else(|| coded("NOT_FOUND", "no such provider"))
    }

    /// Admins only. Usage rows stay (with the provider name).
    async fn delete_provider(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
        require_admin(ctx)?;
        let s = services(ctx);
        let uuid = decode_typed(&s.keys, &id, "Provider")?;
        let deleted = sqlx::query("DELETE FROM llm_providers WHERE id = $1")
            .bind(uuid)
            .execute(&s.pg)
            .await
            .map_err(internal)?;
        forget_models(s).await;
        Ok(deleted.rows_affected() > 0)
    }

    /// A new API key for the viewer. The token is in the answer only: it can't be shown again.
    async fn create_api_key(&self, ctx: &Context<'_>, name: String) -> Result<CreatedApiKey> {
        let viewer = require_viewer(ctx)?;
        let name = name.trim();
        if !(1..=64).contains(&name.chars().count()) {
            return Err(coded("INVALID_NAME", "1–64 characters"));
        }
        let s = services(ctx);
        let token = gateway::new_token();
        let row: ApiKeyRow = sqlx::query_as(
            "INSERT INTO api_keys (user_id, name, token_hash) VALUES ($1, $2, $3)
             RETURNING id, name, created_at, last_used_at",
        )
        .bind(viewer.user_id)
        .bind(name)
        .bind(gateway::hash_token(&token))
        .fetch_one(&s.pg)
        .await
        .map_err(internal)?;
        Ok(CreatedApiKey {
            api_key: ApiKey::from_row(row, &s.keys),
            token,
        })
    }

    /// Revokes one of the viewer's own keys; takes effect on the next request.
    async fn revoke_api_key(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let uuid = decode_typed(&s.keys, &id, "ApiKey")?;
        let revoked = sqlx::query(
            "UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(uuid)
        .bind(viewer.user_id)
        .execute(&s.pg)
        .await
        .map_err(internal)?;
        Ok(revoked.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_and_urls() {
        assert!(validate_name("openai").is_ok());
        assert!(validate_name("my_local-2").is_ok());
        for bad in ["", "OpenAI", "-x", "a/b", "a b", &"x".repeat(33)] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            validate_url("https://api.openai.com/v1/").unwrap(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            validate_url("http://inference:11434/v1").unwrap(),
            "http://inference:11434/v1"
        );
        for bad in ["ftp://x/v1", "not a url", "file:///etc/passwd"] {
            assert!(validate_url(bad).is_err(), "{bad:?}");
        }
    }
}
