//! OpenAI-compatible LLM gateway (`/v1`, REST for machines). Authenticates API keys, routes
//! `"<provider>/<model>"` to that provider's upstream, passes the answer (or SSE stream) through and
//! records token usage.

use std::{
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use anyhow::Context as _;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fred::{prelude::*, types::Expiration};
use futures::{Stream, StreamExt, stream::BoxStream};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{AppState, services::Services};

/// Chat requests carry images and long histories: well above axum's 2 MB default.
const MAX_BODY: usize = 32 << 20;
/// SSE lines longer than this are skipped by the usage scanner instead of buffered.
const MAX_SSE_LINE: usize = 1 << 20;
pub const MODELS_CACHE_KEY: &str = "cache:llm:models";
const MODELS_CACHE_TTL: Duration = Duration::from_secs(60);
pub const TOKEN_PREFIX: &str = "ra_";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/models", get(models))
        .route(
            "/v1/chat/completions",
            post(
                |State(st): State<AppState>, headers: HeaderMap, body: Bytes| async move {
                    forward(&st.services, &headers, &body, "chat/completions").await
                },
            ),
        )
        .route(
            "/v1/embeddings",
            post(
                |State(st): State<AppState>, headers: HeaderMap, body: Bytes| async move {
                    forward(&st.services, &headers, &body, "embeddings").await
                },
            ),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY))
}

// ---- API keys -------------------------------------------------------------------------------

/// A new API key: `ra_` + 244 random bits (two v4 UUIDs, from the OS CSPRNG).
pub fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

/// Tokens are high-entropy, so a plain SHA-256 is enough (no password hashing needed).
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

struct Caller {
    user_id: Uuid,
    key_id: Uuid,
    /// Agent tokens: the agent, and the only model it may use.
    agent: Option<(Uuid, String)>,
}

#[expect(
    clippy::result_large_err,
    reason = "the Err is the response, returned right away"
)]
async fn caller(s: &Services, headers: &HeaderMap) -> Result<Caller, Response> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| t.starts_with(TOKEN_PREFIX));
    let Some(token) = token else {
        return Err(invalid_key());
    };
    let row: Option<(Uuid, Uuid, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "UPDATE api_keys k SET last_used_at = now()
         WHERE token_hash = $1 AND revoked_at IS NULL
         RETURNING k.id, k.user_id, k.agent_id, (SELECT model FROM agents a WHERE a.id = k.agent_id)",
    )
    .bind(hash_token(token))
    .fetch_optional(&s.pg)
    .await
    .map_err(internal)?;
    row.map(|(key_id, user_id, agent_id, model)| Caller {
        user_id,
        key_id,
        agent: agent_id.zip(model),
    })
    .ok_or_else(invalid_key)
}

// ---- Providers ------------------------------------------------------------------------------

/// Encryption context of a provider's API key: binds the ciphertext to its row.
pub fn secret_context(id: Uuid) -> String {
    format!("llm-provider:{id}")
}

pub struct Upstream {
    pub id: Uuid,
    pub name: String,
    base_url: String,
    api_key: Option<String>,
}

/// Enabled providers, or only the one named `name`.
pub async fn upstreams(s: &Services, name: Option<&str>) -> anyhow::Result<Vec<Upstream>> {
    let rows: Vec<(Uuid, String, String, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT id, name, base_url, api_key FROM llm_providers
         WHERE enabled AND ($1::text IS NULL OR name = $1) ORDER BY name",
    )
    .bind(name)
    .fetch_all(&s.pg)
    .await?;
    rows.into_iter()
        .map(|(id, name, base_url, sealed)| {
            let api_key = sealed
                .map(|c| {
                    let plain = s.keys.decrypt_secret(&secret_context(id), &c)?;
                    Ok::<_, anyhow::Error>(String::from_utf8(plain)?)
                })
                .transpose()?;
            Ok(Upstream {
                id,
                name,
                base_url,
                api_key,
            })
        })
        .collect()
}

impl Upstream {
    /// Also used for providers being edited, before they are saved or while disabled.
    pub fn new(id: Uuid, name: String, base_url: String, api_key: Option<String>) -> Self {
        Self {
            id,
            name,
            base_url,
            api_key,
        }
    }

    fn request(&self, s: &Services, method: Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{path}", self.base_url.trim_end_matches('/'));
        let req = s.llm.request(method, url);
        match &self.api_key {
            Some(key) => req.bearer_auth(key),
            None => req,
        }
    }

    /// The provider's model ids, from its own `/models`.
    pub async fn models(&self, s: &Services) -> anyhow::Result<Vec<String>> {
        let v: Value = self
            .request(s, Method::GET, "models")
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut ids: Vec<String> = v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().map(String::from))
            .collect();
        ids.sort();
        Ok(ids)
    }
}

async fn models(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let s = &st.services;
    let caller = match caller(s, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match list_models(s).await {
        Ok(Value::Array(mut data)) => {
            if let Some((_, model)) = &caller.agent {
                data.retain(|m| m["id"].as_str() == Some(model.as_str()));
            }
            Json(json!({ "object": "list", "data": data })).into_response()
        }
        Ok(data) => Json(json!({ "object": "list", "data": data })).into_response(),
        Err(e) => internal(e),
    }
}

/// Every enabled provider's models, prefixed with the provider name. Cached briefly in Valkey
/// (TTL'd, so evictable); provider mutations delete the cache.
pub async fn list_models(s: &Services) -> anyhow::Result<Value> {
    if let Ok(Some(cached)) = s.valkey.get::<Option<String>, _>(MODELS_CACHE_KEY).await
        && let Ok(v) = serde_json::from_str(&cached)
    {
        return Ok(v);
    }
    let ups = upstreams(s, None).await?;
    let lists = futures::future::join_all(ups.iter().map(|u| u.models(s))).await;
    let mut data = Vec::new();
    for (up, list) in ups.iter().zip(lists) {
        match list {
            Ok(ids) => data.extend(ids.into_iter().map(|id| {
                json!({ "id": format!("{}/{id}", up.name), "object": "model", "owned_by": up.name })
            })),
            // One provider down must not hide the others' models.
            Err(e) => tracing::warn!("llm provider {}: listing models failed: {e:#}", up.name),
        }
    }
    let data = Value::Array(data);
    let cached: Result<(), _> = s
        .valkey
        .set(
            MODELS_CACHE_KEY,
            data.to_string(),
            Some(Expiration::EX(MODELS_CACHE_TTL.as_secs() as i64)),
            None,
            false,
        )
        .await;
    if let Err(e) = cached {
        tracing::warn!("caching llm models: {e}");
    }
    Ok(data)
}

// ---- Forwarding -----------------------------------------------------------------------------

/// `"openai/gpt-4o"` → `("openai", "gpt-4o")`. Model ids may contain `/` themselves (OpenRouter).
fn split_model(model: &str) -> Option<(&str, &str)> {
    model
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
}

async fn forward(
    s: &Services,
    headers: &HeaderMap,
    body: &[u8],
    endpoint: &'static str,
) -> Response {
    let caller = match caller(s, headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let Ok(Value::Object(mut req)) = serde_json::from_slice(body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "the body must be a JSON object",
        );
    };
    if let Some((_, allowed)) = &caller.agent
        && req.get("model").and_then(Value::as_str) != Some(allowed.as_str())
    {
        return error(
            StatusCode::FORBIDDEN,
            "model_not_allowed",
            "this agent token may only use its agent's model",
        );
    }
    let Some((provider, model)) = req
        .get("model")
        .and_then(Value::as_str)
        .and_then(split_model)
        .map(|(p, m)| (p.to_owned(), m.to_owned()))
    else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "model must be \"<provider>/<model>\"",
        );
    };
    let up = match upstreams(s, Some(&provider)).await {
        Ok(mut ups) => match ups.pop() {
            Some(up) => up,
            None => {
                return error(
                    StatusCode::NOT_FOUND,
                    "model_not_found",
                    "unknown or disabled provider",
                );
            }
        },
        Err(e) => return internal(e),
    };

    req.insert("model".into(), model.clone().into());
    let stream = req.get("stream").and_then(Value::as_bool) == Some(true);
    if stream {
        // Ask for the final usage chunk so streamed requests are metered too.
        let opts = req.entry("stream_options").or_insert_with(|| json!({}));
        if let Some(opts) = opts.as_object_mut() {
            opts.insert("include_usage".into(), true.into());
        }
    }

    let usage = UsageRecord {
        user_id: Some(caller.user_id),
        key_id: Some(caller.key_id),
        agent_id: caller.agent.map(|(id, _)| id),
        provider_id: up.id,
        provider,
        model,
        endpoint,
        started: Instant::now(),
    };
    let resp = match up
        .request(s, Method::POST, endpoint)
        .json(&req)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("llm provider {}: {e}", up.name);
            usage.record(s, StatusCode::BAD_GATEWAY, None);
            return error(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "the provider could not be reached",
            );
        }
    };

    let status = resp.status();
    let mut out = Response::builder().status(status);
    if let Some(ct) = resp.headers().get(header::CONTENT_TYPE) {
        out = out.header(header::CONTENT_TYPE, ct);
    }
    if stream && status.is_success() {
        let body = Metered {
            inner: resp.bytes_stream().boxed(),
            scanner: UsageScanner::default(),
            usage: Some(usage),
            services: s.clone(),
            status,
            done: false,
        };
        return out
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(body))
            .expect("valid response");
    }
    match resp.bytes().await {
        Ok(bytes) => {
            let tokens = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| Tokens::from_usage(&v["usage"]));
            usage.record(s, status, tokens);
            out.body(Body::from(bytes)).expect("valid response")
        }
        Err(e) => {
            tracing::warn!("llm provider {}: reading the answer: {e}", up.name);
            usage.record(s, StatusCode::BAD_GATEWAY, None);
            error(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "the provider's answer was cut off",
            )
        }
    }
}

/// One non-streamed chat completion for an in-process agent, metered like `/v1` calls. Error messages are
/// shown in the channel, so they never include upstream bodies or URLs.
pub async fn complete(
    s: &Services,
    model: &str,
    messages: Value,
    user_id: Option<Uuid>,
    agent_id: Uuid,
) -> anyhow::Result<String> {
    let (provider, model) =
        split_model(model).context("the agent's model must be \"<provider>/<model>\"")?;
    let up = upstreams(s, Some(provider))
        .await?
        .pop()
        .with_context(|| format!("provider \"{provider}\" is unknown or disabled"))?;
    let usage = UsageRecord {
        user_id,
        key_id: None,
        agent_id: Some(agent_id),
        provider_id: up.id,
        provider: provider.to_owned(),
        model: model.to_owned(),
        endpoint: "chat/completions",
        started: Instant::now(),
    };
    let sent = up
        .request(s, Method::POST, "chat/completions")
        .json(&json!({ "model": model, "messages": messages }))
        .send()
        .await;
    let resp = match sent {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("llm provider {}: {e}", up.name);
            usage.record(s, StatusCode::BAD_GATEWAY, None);
            anyhow::bail!("provider \"{provider}\" could not be reached");
        }
    };
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_default();
    usage.record(s, status, Tokens::from_usage(&body["usage"]));
    if !status.is_success() {
        tracing::warn!("llm provider {}: {status}: {body}", up.name);
        anyhow::bail!("provider \"{provider}\" answered {status}");
    }
    body["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_owned)
        .context("the provider's answer had no message")
}

// ---- Usage ----------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
struct Tokens {
    prompt: Option<i32>,
    completion: Option<i32>,
}

impl Tokens {
    /// From an OpenAI `usage` object.
    fn from_usage(usage: &Value) -> Option<Self> {
        let n = |k: &str| usage[k].as_i64().and_then(|n| i32::try_from(n).ok());
        usage.is_object().then(|| Self {
            prompt: n("prompt_tokens"),
            completion: n("completion_tokens"),
        })
    }
}

struct UsageRecord {
    user_id: Option<Uuid>,
    key_id: Option<Uuid>,
    agent_id: Option<Uuid>,
    provider_id: Uuid,
    provider: String,
    model: String,
    endpoint: &'static str,
    started: Instant,
}

impl UsageRecord {
    // shortcut: a fire-and-forget insert; a crash between answer and insert loses that row. Queue it (Valkey Stream) if billing depends on it.
    fn record(self, s: &Services, status: StatusCode, tokens: Option<Tokens>) {
        let pg = s.pg.clone();
        let ms = i32::try_from(self.started.elapsed().as_millis()).unwrap_or(i32::MAX);
        let tokens = tokens.unwrap_or(Tokens {
            prompt: None,
            completion: None,
        });
        tokio::spawn(async move {
            let inserted = sqlx::query(
                "INSERT INTO llm_usage (user_id, api_key_id, provider_id, provider, model, endpoint, status,
                                        prompt_tokens, completion_tokens, duration_ms, agent_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(self.user_id)
            .bind(self.key_id)
            .bind(self.provider_id)
            .bind(self.provider)
            .bind(self.model)
            .bind(self.endpoint)
            .bind(i32::from(status.as_u16()))
            .bind(tokens.prompt)
            .bind(tokens.completion)
            .bind(ms)
            .bind(self.agent_id)
            .execute(&pg)
            .await;
            if let Err(e) = inserted {
                tracing::warn!("recording llm usage: {e}");
            }
        });
    }
}

/// Finds the `usage` object in an SSE stream (`data: {...}` lines), across chunk boundaries.
#[derive(Default)]
struct UsageScanner {
    line: Vec<u8>,
    overflow: bool,
    usage: Option<Value>,
}

impl UsageScanner {
    fn feed(&mut self, chunk: &[u8]) {
        for part in chunk.split_inclusive(|&b| b == b'\n') {
            let (bytes, ended) = match part.strip_suffix(b"\n") {
                Some(b) => (b, true),
                None => (part, false),
            };
            if self.line.len() + bytes.len() > MAX_SSE_LINE {
                self.overflow = true;
            } else if !self.overflow {
                self.line.extend_from_slice(bytes);
            }
            if ended {
                self.end_line();
            }
        }
    }

    fn end_line(&mut self) {
        let line = std::mem::take(&mut self.line);
        if std::mem::take(&mut self.overflow) {
            return;
        }
        let Some(data) = line.strip_prefix(b"data:") else {
            return;
        };
        if let Ok(mut v) = serde_json::from_slice::<Value>(data.trim_ascii())
            && v["usage"].is_object()
        {
            self.usage = Some(v["usage"].take());
        }
    }
}

/// Passes the upstream SSE stream through, then records usage once it ends or the client leaves.
struct Metered {
    inner: BoxStream<'static, reqwest::Result<Bytes>>,
    scanner: UsageScanner,
    usage: Option<UsageRecord>,
    services: Services,
    status: StatusCode,
    done: bool,
}

impl Stream for Metered {
    type Item = reqwest::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        let poll = this.inner.poll_next_unpin(cx);
        match &poll {
            Poll::Ready(Some(Ok(chunk))) => this.scanner.feed(chunk),
            Poll::Ready(None) => this.done = true,
            _ => {}
        }
        poll
    }
}

impl Drop for Metered {
    fn drop(&mut self) {
        if let Some(usage) = self.usage.take() {
            // 499: the client closed the connection before the end (nginx's convention).
            let status = if self.done {
                self.status
            } else {
                StatusCode::from_u16(499).expect("valid status")
            };
            let tokens = self.scanner.usage.as_ref().and_then(Tokens::from_usage);
            usage.record(&self.services, status, tokens);
        }
    }
}

// ---- Errors ---------------------------------------------------------------------------------

/// OpenAI-shaped error body, so OpenAI SDKs surface it properly.
fn error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "message": message, "type": kind, "code": kind } })),
    )
        .into_response()
}

fn invalid_key() -> Response {
    error(
        StatusCode::UNAUTHORIZED,
        "invalid_api_key",
        "missing, invalid or revoked API key",
    )
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::error!("llm gateway: {e:#}");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "internal error",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_model_keeps_slashes_in_the_model() {
        assert_eq!(split_model("openai/gpt-4o"), Some(("openai", "gpt-4o")));
        assert_eq!(
            split_model("openrouter/anthropic/claude"),
            Some(("openrouter", "anthropic/claude"))
        );
        assert_eq!(split_model("gpt-4o"), None);
        assert_eq!(split_model("/gpt"), None);
        assert_eq!(split_model("openai/"), None);
    }

    #[test]
    fn scanner_finds_usage_across_chunks() {
        let sse = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n\
                    data: [DONE]\n\n";
        for split in [1, 7, 40, sse.len()] {
            let mut scanner = UsageScanner::default();
            sse.chunks(split).for_each(|c| scanner.feed(c));
            let tokens = scanner.usage.as_ref().and_then(Tokens::from_usage);
            assert_eq!(
                tokens,
                Some(Tokens {
                    prompt: Some(12),
                    completion: Some(3)
                }),
                "chunk size {split}"
            );
        }
    }

    #[test]
    fn scanner_skips_oversized_lines_and_null_usage() {
        let mut scanner = UsageScanner::default();
        scanner.feed(b"data: {\"usage\":null}\n");
        scanner.feed(&vec![b'x'; MAX_SSE_LINE + 1]);
        scanner.feed(b"\ndata: {\"usage\":{\"prompt_tokens\":1}}\r\n");
        assert_eq!(scanner.usage, Some(json!({"prompt_tokens": 1})));
    }

    #[test]
    fn tokens_are_hashed_and_prefixed() {
        let t = new_token();
        assert!(t.starts_with(TOKEN_PREFIX) && t.len() == 3 + 64);
        assert_ne!(t, new_token());
        assert_eq!(hash_token(&t), hash_token(&t));
    }
}
