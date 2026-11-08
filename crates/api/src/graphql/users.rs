use async_graphql::{
    Context, ID, Interface, Object, Result, SimpleObject,
    connection::{Connection, Edge, query},
};
use chrono::{DateTime, Utc};
use crypto::{Keys, TokenError};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, types::Json};
use uuid::Uuid;

use super::{ID_TTL, auth_error, coded, require_admin, require_viewer, services};
use crate::auth;

/// Relay object identification: `id` is an expiring encrypted token, `cacheKey` the stable client identity.
#[derive(Interface)]
#[graphql(
    field(name = "id", ty = "&ID"),
    field(name = "cache_key", ty = "&String")
)]
pub enum Node {
    User(User),
}

#[derive(SimpleObject, Clone)]
pub struct User {
    pub id: ID,
    pub cache_key: String,
    pub username: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub prefs: Prefs,
}

/// UI preferences, stored per user so they follow them across devices.
#[derive(SimpleObject, Clone, Default, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: Option<String>,
    pub language: Option<String>,
}

const THEMES: &[&str] = &["system", "light", "dark"];
const LANGUAGES: &[&str] = &["en", "fr"];

#[derive(FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
    is_admin: bool,
    created_at: DateTime<Utc>,
    prefs: Json<Prefs>,
}

impl User {
    fn from_row(row: UserRow, keys: &Keys) -> Self {
        Self {
            id: ID(keys.encode_id("User", row.id, ID_TTL)),
            cache_key: keys.cache_key("User", row.id),
            username: row.username,
            is_admin: row.is_admin,
            created_at: row.created_at,
            prefs: row.prefs.0,
        }
    }
}

/// Columns of `UserRow`, as a literal so queries stay static SQL.
macro_rules! user_columns {
    () => {
        "id, username, is_admin, created_at, prefs"
    };
}

async fn load_user(pg: &PgPool, keys: &Keys, id: Uuid) -> Result<Option<User>> {
    let row: Option<UserRow> = sqlx::query_as(concat!(
        "SELECT ",
        user_columns!(),
        " FROM users WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pg)
    .await?;
    Ok(row.map(|r| User::from_row(r, keys)))
}

/// Decodes a client ID, mapping failures to the codes the UI handles.
fn decode_id(keys: &Keys, id: &str) -> Result<(String, Uuid)> {
    keys.decode_id(id).map_err(|e| match e {
        TokenError::Expired => coded("ID_EXPIRED", "id expired, refetch it"),
        TokenError::Invalid => coded("INVALID_ID", "invalid id"),
    })
}

#[derive(Default)]
pub struct UsersQuery;

#[Object]
impl UsersQuery {
    /// True until the first admin is created (see the setup code in the server logs).
    async fn setup_required(&self, ctx: &Context<'_>) -> Result<bool> {
        Ok(!auth::has_users(&services(ctx).pg)
            .await
            .map_err(auth_error)?)
    }

    /// The signed-in user, or null.
    async fn viewer(&self, ctx: &Context<'_>) -> Result<Option<User>> {
        let Ok(viewer) = require_viewer(ctx) else {
            return Ok(None);
        };
        let s = services(ctx);
        load_user(&s.pg, &s.keys, viewer.user_id).await
    }

    /// Refetches any object by its Relay ID. Returns null for objects the viewer may not see.
    async fn node(&self, ctx: &Context<'_>, id: ID) -> Result<Option<Node>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let (type_name, uuid) = decode_id(&s.keys, &id)?;
        match type_name.as_str() {
            "User" if viewer.is_admin || viewer.user_id == uuid => {
                Ok(load_user(&s.pg, &s.keys, uuid).await?.map(Node::User))
            }
            _ => Ok(None),
        }
    }

    /// All users, oldest first. Admins only.
    async fn users(
        &self,
        ctx: &Context<'_>,
        after: Option<String>,
        first: Option<i32>,
    ) -> Result<Connection<String, User>> {
        require_admin(ctx)?;
        let s = services(ctx);
        query(
            after,
            None,
            first,
            None,
            |after: Option<String>, _, first, _| async move {
                let after = after
                    .map(|c| decode_id(&s.keys, &c))
                    .transpose()?
                    .map(|(_, id)| id);
                let limit = first.unwrap_or(50).min(100);
                // UUIDv7 ids sort by creation time: keyset pagination on id.
                let rows: Vec<UserRow> = sqlx::query_as(concat!(
                    "SELECT ",
                    user_columns!(),
                    " FROM users WHERE $1::uuid IS NULL OR id > $1 ORDER BY id LIMIT $2"
                ))
                .bind(after)
                .bind(limit as i64 + 1)
                .fetch_all(&s.pg)
                .await?;
                let mut connection = Connection::new(after.is_some(), rows.len() > limit);
                connection
                    .edges
                    .extend(rows.into_iter().take(limit).map(|row| {
                        let cursor = s.keys.encode_id("User", row.id, ID_TTL);
                        Edge::new(cursor, User::from_row(row, &s.keys))
                    }));
                Ok::<_, async_graphql::Error>(connection)
            },
        )
        .await
    }
}

#[derive(Default)]
pub struct UsersMutation;

#[Object]
impl UsersMutation {
    /// Creates the first admin with the one-time setup code from the server logs, and signs them in.
    async fn setup(
        &self,
        ctx: &Context<'_>,
        code: String,
        username: String,
        password: String,
    ) -> Result<User> {
        let s = services(ctx);
        let expected = s.setup_code.lock().expect("setup code lock").clone();
        let id = auth::setup(&s.pg, expected.as_deref(), &code, &username, &password)
            .await
            .map_err(auth_error)?;
        *s.setup_code.lock().expect("setup code lock") = None;
        sign_in(ctx, id).await
    }

    async fn login(&self, ctx: &Context<'_>, username: String, password: String) -> Result<User> {
        let s = services(ctx);
        let id = auth::login(&s.pg, &s.valkey, &username, &password)
            .await
            .map_err(auth_error)?;
        sign_in(ctx, id).await
    }

    async fn logout(&self, ctx: &Context<'_>) -> Result<bool> {
        if let Ok(viewer) = require_viewer(ctx) {
            auth::revoke_session(&services(ctx).pg, viewer.session_id)
                .await
                .map_err(auth_error)?;
        }
        ctx.append_http_header(
            "Set-Cookie",
            auth::clear_cookie(services(ctx).cfg.cookie_secure),
        );
        Ok(true)
    }

    /// Also signs out every other session of the user.
    async fn change_password(
        &self,
        ctx: &Context<'_>,
        current: String,
        new: String,
    ) -> Result<bool> {
        let viewer = require_viewer(ctx)?;
        auth::change_password(&services(ctx).pg, &viewer, &current, &new)
            .await
            .map_err(auth_error)?;
        Ok(true)
    }

    /// Sets the given preferences; omitted ones keep their value.
    async fn update_prefs(
        &self,
        ctx: &Context<'_>,
        theme: Option<String>,
        language: Option<String>,
    ) -> Result<User> {
        let viewer = require_viewer(ctx)?;
        if theme.as_deref().is_some_and(|t| !THEMES.contains(&t)) {
            return Err(coded("INVALID_PREF", "unknown theme"));
        }
        if language.as_deref().is_some_and(|l| !LANGUAGES.contains(&l)) {
            return Err(coded("INVALID_PREF", "unknown language"));
        }
        let s = services(ctx);
        let mut patch = serde_json::Map::new();
        if let Some(t) = theme {
            patch.insert("theme".into(), t.into());
        }
        if let Some(l) = language {
            patch.insert("language".into(), l.into());
        }
        sqlx::query("UPDATE users SET prefs = prefs || $1 WHERE id = $2")
            .bind(Json(patch))
            .bind(viewer.user_id)
            .execute(&s.pg)
            .await?;
        load_user(&s.pg, &s.keys, viewer.user_id)
            .await?
            .ok_or_else(|| "user not found".into())
    }

    /// Admins only.
    async fn create_user(
        &self,
        ctx: &Context<'_>,
        username: String,
        password: String,
        is_admin: bool,
    ) -> Result<User> {
        require_admin(ctx)?;
        let s = services(ctx);
        let id = auth::create_user(&s.pg, &username, &password, is_admin)
            .await
            .map_err(auth_error)?;
        load_user(&s.pg, &s.keys, id)
            .await?
            .ok_or_else(|| "user not found".into())
    }
}

/// Opens a session and sets the cookie on the response.
async fn sign_in(ctx: &Context<'_>, user_id: Uuid) -> Result<User> {
    let s = services(ctx);
    let token = auth::create_session(&s.pg, &s.keys, user_id)
        .await
        .map_err(auth_error)?;
    ctx.append_http_header(
        "Set-Cookie",
        auth::session_cookie(&token, s.cfg.cookie_secure),
    );
    load_user(&s.pg, &s.keys, user_id)
        .await?
        .ok_or_else(|| "user not found".into())
}
