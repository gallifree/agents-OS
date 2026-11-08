//! Users, passwords, login lockout and sessions. GraphQL resolvers stay thin and call into this.

use std::{sync::LazyLock, time::Duration};

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use chrono::Utc;
use crypto::Keys;
use fred::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
pub const COOKIE_NAME: &str = "session";
const MAX_FAILED_LOGINS: i64 = 10;
const LOCKOUT_SECS: i64 = 15 * 60;
/// Argon2 cost grows with input length: cap it so a huge password can't burn the CPU.
const MAX_PASSWORD_LEN: usize = 1024;
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("too many failed attempts, try again later")]
    LockedOut,
    #[error("username already taken")]
    UsernameTaken,
    #[error("username must be 3–32 characters: letters, digits, '.', '_' or '-'")]
    InvalidUsername,
    #[error("password must be {MIN_PASSWORD_LEN}–{MAX_PASSWORD_LEN} characters")]
    InvalidPassword,
    #[error("invalid setup code")]
    InvalidSetupCode,
    #[error("setup already done")]
    SetupDone,
    #[error(transparent)]
    Internal(#[from] InternalError),
}

/// Infrastructure failures; their details are logged, never sent to clients.
#[derive(Debug, thiserror::Error)]
#[error("internal error")]
pub struct InternalError(#[source] anyhow::Error);

impl AuthError {
    /// Stable machine-readable code for GraphQL error extensions (the UI translates codes, not messages).
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidCredentials => "INVALID_CREDENTIALS",
            Self::LockedOut => "LOCKED_OUT",
            Self::UsernameTaken => "USERNAME_TAKEN",
            Self::InvalidUsername => "INVALID_USERNAME",
            Self::InvalidPassword => "INVALID_PASSWORD",
            Self::InvalidSetupCode => "INVALID_SETUP_CODE",
            Self::SetupDone => "SETUP_DONE",
            Self::Internal(_) => "INTERNAL",
        }
    }
}

fn internal(e: impl Into<anyhow::Error>) -> AuthError {
    let e = e.into();
    tracing::error!("auth: {e:#}");
    AuthError::Internal(InternalError(e))
}

/// The authenticated caller of a request.
#[derive(Clone, Copy, Debug)]
pub struct Viewer {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub is_admin: bool,
}

// ---- Validation & hashing -------------------------------------------------------------------

pub fn normalize_username(raw: &str) -> Result<String, AuthError> {
    let name = raw.trim().to_lowercase();
    let valid = (3..=32).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(name)
    } else {
        Err(AuthError::InvalidUsername)
    }
}

pub fn validate_password(password: &str) -> Result<(), AuthError> {
    if (MIN_PASSWORD_LEN..=MAX_PASSWORD_LEN).contains(&password.chars().count()) {
        Ok(())
    } else {
        Err(AuthError::InvalidPassword)
    }
}

/// Argon2 is deliberately slow: run it off the async workers.
async fn hash_password(password: String) -> Result<String, AuthError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
    })
    .await
    .map_err(internal)?
    .map_err(|e| internal(anyhow::anyhow!("argon2: {e}")))
}

async fn verify_password(password: String, hash: String) -> Result<bool, AuthError> {
    tokio::task::spawn_blocking(move || {
        let parsed = PasswordHash::new(&hash).map_err(|e| anyhow::anyhow!("stored hash: {e}"))?;
        Ok::<_, anyhow::Error>(
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok(),
        )
    })
    .await
    .map_err(internal)?
    .map_err(internal)
}

/// Verified when the username doesn't exist, so response time doesn't reveal which usernames exist.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"dummy password for timing")
        .expect("argon2")
        .to_string()
});

// ---- Users ----------------------------------------------------------------------------------

pub async fn create_user(
    pg: &PgPool,
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<Uuid, AuthError> {
    let username = normalize_username(username)?;
    validate_password(password)?;
    let hash = hash_password(password.to_string()).await?;
    sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, is_admin) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&username)
    .bind(&hash)
    .bind(is_admin)
    .fetch_one(pg)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => AuthError::UsernameTaken,
        _ => internal(e),
    })
}

pub async fn has_users(pg: &PgPool) -> Result<bool, AuthError> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(pg)
        .await
        .map_err(internal)
}

/// Creates the first admin. `expected_code` is the one-time code logged at startup.
pub async fn setup(
    pg: &PgPool,
    expected_code: Option<&str>,
    code: &str,
    username: &str,
    password: &str,
) -> Result<Uuid, AuthError> {
    // Compare digests rather than the codes themselves, so timing reveals nothing usable.
    let matches = expected_code.is_some_and(|expected| {
        Sha256::digest(expected.trim().to_lowercase()) == Sha256::digest(code.trim().to_lowercase())
    });
    if !matches {
        return Err(AuthError::InvalidSetupCode);
    }
    let username = normalize_username(username)?;
    validate_password(password)?;
    let hash = hash_password(password.to_string()).await?;

    // The advisory lock serializes concurrent setups: only the first one sees an empty table.
    let mut tx = pg.begin().await.map_err(internal)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('rust-agents:setup'))")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if exists {
        return Err(AuthError::SetupDone);
    }
    let id = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, is_admin) VALUES ($1, $2, true) RETURNING id",
    )
    .bind(&username)
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(id)
}

/// Checks credentials, with a per-username lockout after repeated failures.
// shortcut: lockout is per username, so someone can lock a known account out for 15 min; add per-IP limits behind a proxy.
pub async fn login(
    pg: &PgPool,
    valkey: &Client,
    username: &str,
    password: &str,
) -> Result<Uuid, AuthError> {
    let username = normalize_username(username).map_err(|_| AuthError::InvalidCredentials)?;
    if password.chars().count() > MAX_PASSWORD_LEN {
        return Err(AuthError::InvalidCredentials);
    }
    let fail_key = format!("login:fail:{username}");
    let failures: Option<i64> = valkey.get(&fail_key).await.map_err(internal)?;
    if failures.unwrap_or(0) >= MAX_FAILED_LOGINS {
        return Err(AuthError::LockedOut);
    }

    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = $1")
            .bind(&username)
            .fetch_optional(pg)
            .await
            .map_err(internal)?;
    let (id, hash) = match row {
        Some((id, hash)) => (Some(id), hash),
        None => (None, DUMMY_HASH.clone()),
    };
    let valid = verify_password(password.to_string(), hash).await?;
    match id {
        Some(id) if valid => {
            let _: i64 = valkey.del(&fail_key).await.map_err(internal)?;
            Ok(id)
        }
        _ => {
            let count: i64 = valkey.incr(&fail_key).await.map_err(internal)?;
            if count == 1 {
                let _: bool = valkey
                    .expire(&fail_key, LOCKOUT_SECS, None)
                    .await
                    .map_err(internal)?;
            }
            Err(AuthError::InvalidCredentials)
        }
    }
}

/// Changes the password and ends every other session of the user.
pub async fn change_password(
    pg: &PgPool,
    viewer: &Viewer,
    current: &str,
    new: &str,
) -> Result<(), AuthError> {
    validate_password(new)?;
    if current.chars().count() > MAX_PASSWORD_LEN {
        return Err(AuthError::InvalidCredentials);
    }
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(viewer.user_id)
        .fetch_one(pg)
        .await
        .map_err(internal)?;
    if !verify_password(current.to_string(), hash).await? {
        return Err(AuthError::InvalidCredentials);
    }
    let new_hash = hash_password(new.to_string()).await?;
    let mut tx = pg.begin().await.map_err(internal)?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&new_hash)
        .bind(viewer.user_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL")
        .bind(viewer.user_id)
        .bind(viewer.session_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)
}

// ---- Sessions -------------------------------------------------------------------------------

/// Returns the encrypted session token to put in the cookie.
pub async fn create_session(pg: &PgPool, keys: &Keys, user_id: Uuid) -> Result<String, AuthError> {
    let expires_at = Utc::now() + SESSION_TTL;
    let session_id: Uuid = sqlx::query_scalar(
        "INSERT INTO sessions (user_id, expires_at) VALUES ($1, $2) RETURNING id",
    )
    .bind(user_id)
    .bind(expires_at)
    .fetch_one(pg)
    .await
    .map_err(internal)?;
    Ok(keys.encode_session(session_id, user_id, SESSION_TTL))
}

/// The token's expiry and signature are checked first; the database row decides revocation.
pub async fn resolve_session(
    pg: &PgPool,
    keys: &Keys,
    token: &str,
) -> Result<Option<Viewer>, AuthError> {
    let Ok((session_id, user_id)) = keys.decode_session(token) else {
        return Ok(None);
    };
    let is_admin: Option<bool> = sqlx::query_scalar(
        "SELECT u.is_admin FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.id = $1 AND s.user_id = $2 AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(pg)
    .await
    .map_err(internal)?;
    Ok(is_admin.map(|is_admin| Viewer {
        user_id,
        session_id,
        is_admin,
    }))
}

pub async fn revoke_session(pg: &PgPool, session_id: Uuid) -> Result<(), AuthError> {
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
        .bind(session_id)
        .execute(pg)
        .await
        .map_err(internal)?;
    Ok(())
}

// ---- Cookie ---------------------------------------------------------------------------------

/// `SameSite=Strict` keeps the cookie off every cross-site request, WebSocket upgrades included.
pub fn session_cookie(token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
        SESSION_TTL.as_secs()
    )
}

pub fn clear_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
}

/// Reads our cookie from a `Cookie` header value.
pub fn cookie_token(header: &str) -> Option<&str> {
    header
        .split(';')
        .find_map(|c| c.trim().strip_prefix(COOKIE_NAME)?.strip_prefix('='))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_normalized_and_validated() {
        assert_eq!(normalize_username("  Jane.D ").unwrap(), "jane.d");
        assert!(matches!(
            normalize_username("ab"),
            Err(AuthError::InvalidUsername)
        ));
        assert!(matches!(
            normalize_username("has space"),
            Err(AuthError::InvalidUsername)
        ));
        assert!(matches!(
            normalize_username("élodie"),
            Err(AuthError::InvalidUsername)
        ));
        assert!(matches!(
            normalize_username(&"a".repeat(33)),
            Err(AuthError::InvalidUsername)
        ));
    }

    #[test]
    fn password_length_is_bounded() {
        assert!(matches!(
            validate_password("short"),
            Err(AuthError::InvalidPassword)
        ));
        assert!(validate_password("long enough").is_ok());
        assert!(matches!(
            validate_password(&"x".repeat(MAX_PASSWORD_LEN + 1)),
            Err(AuthError::InvalidPassword)
        ));
    }

    #[test]
    fn reads_only_our_cookie() {
        assert_eq!(cookie_token("theme=dark; session=abc; x=1"), Some("abc"));
        assert_eq!(cookie_token("sessionx=abc"), None);
        assert_eq!(cookie_token("other=1"), None);
        assert!(session_cookie("t", true).ends_with("; Secure"));
        assert!(!clear_cookie(false).contains("Secure"));
    }
}

/// Runs against real Postgres and Valkey containers (needs Docker).
#[cfg(test)]
mod db_tests {
    use std::time::Duration;

    use testcontainers::{
        ContainerAsync, GenericImage, ImageExt,
        core::{IntoContainerPort, WaitFor},
        runners::AsyncRunner,
    };

    use super::*;

    async fn postgres() -> (ContainerAsync<GenericImage>, PgPool) {
        let container = GenericImage::new("postgres", "18")
            .with_exposed_port(5432.tcp())
            .with_wait_for(WaitFor::message_on_stderr("ready to accept connections"))
            .with_env_var("POSTGRES_PASSWORD", "test")
            .start()
            .await
            .expect("start postgres");
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:test@127.0.0.1:{port}/postgres");
        // Postgres restarts once after initdb: retry until it really accepts connections.
        for _ in 0..50 {
            if let Ok(pg) = PgPool::connect(&url).await {
                sqlx::migrate!().run(&pg).await.expect("migrations");
                return (container, pg);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("postgres never became ready");
    }

    async fn valkey() -> (ContainerAsync<GenericImage>, Client) {
        let container = GenericImage::new("valkey/valkey", "9")
            .with_exposed_port(6379.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
            .start()
            .await
            .expect("start valkey");
        let port = container.get_host_port_ipv4(6379).await.unwrap();
        let config =
            fred::types::config::Config::from_url(&format!("redis://127.0.0.1:{port}")).unwrap();
        let client = Builder::from_config(config).build().unwrap();
        client.init().await.unwrap();
        (container, client)
    }

    #[tokio::test]
    async fn auth_flow_against_real_postgres_and_valkey() {
        let ((_pg_c, pg), (_vk_c, vk)) = tokio::join!(postgres(), valkey());
        // base64 of 32 bytes of 0x07
        let keys = Keys::from_master("BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=").unwrap();

        // Setup: needs the code, creates an admin once.
        assert!(!has_users(&pg).await.unwrap());
        const CODE: &str = "abcd-ef01-2345-6789";
        let code = Some(CODE);
        assert!(matches!(
            setup(&pg, code, "nope", "admin", "password1").await,
            Err(AuthError::InvalidSetupCode)
        ));
        assert!(matches!(
            setup(&pg, None, "", "admin", "password1").await,
            Err(AuthError::InvalidSetupCode)
        ));
        let admin = setup(&pg, code, " ABCD-EF01-2345-6789 ", "Admin", "password1")
            .await
            .unwrap();
        assert!(matches!(
            setup(&pg, code, CODE, "other", "password1").await,
            Err(AuthError::SetupDone)
        ));

        // Usernames are unique case-insensitively.
        let bob = create_user(&pg, "bob", "password2", false).await.unwrap();
        assert!(matches!(
            create_user(&pg, "BOB", "password2", false).await,
            Err(AuthError::UsernameTaken)
        ));

        // Login.
        assert_eq!(login(&pg, &vk, "ADMIN", "password1").await.unwrap(), admin);
        assert!(matches!(
            login(&pg, &vk, "admin", "wrong-pass").await,
            Err(AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            login(&pg, &vk, "ghost", "password1").await,
            Err(AuthError::InvalidCredentials)
        ));

        // Lockout after repeated failures, even with the right password afterwards.
        for _ in 0..MAX_FAILED_LOGINS {
            let _ = login(&pg, &vk, "bob", "wrong-pass").await;
        }
        assert!(matches!(
            login(&pg, &vk, "bob", "password2").await,
            Err(AuthError::LockedOut)
        ));
        let ttl: i64 = vk.ttl("login:fail:bob").await.unwrap();
        assert!(ttl > 0 && ttl <= LOCKOUT_SECS, "lockout expires");

        // Sessions resolve to the right viewer and stop working once revoked or tampered with.
        let token = create_session(&pg, &keys, admin).await.unwrap();
        let viewer = resolve_session(&pg, &keys, &token)
            .await
            .unwrap()
            .expect("valid session");
        assert_eq!(viewer.user_id, admin);
        assert!(viewer.is_admin);
        assert!(
            resolve_session(&pg, &keys, &format!("{token}x"))
                .await
                .unwrap()
                .is_none()
        );
        revoke_session(&pg, viewer.session_id).await.unwrap();
        assert!(resolve_session(&pg, &keys, &token).await.unwrap().is_none());

        // Changing the password checks the current one and signs out the other sessions only.
        let current = create_session(&pg, &keys, bob).await.unwrap();
        let other = create_session(&pg, &keys, bob).await.unwrap();
        let bob_viewer = resolve_session(&pg, &keys, &current)
            .await
            .unwrap()
            .unwrap();
        assert!(!bob_viewer.is_admin);
        assert!(matches!(
            change_password(&pg, &bob_viewer, "wrong-pass", "new-password").await,
            Err(AuthError::InvalidCredentials)
        ));
        change_password(&pg, &bob_viewer, "password2", "new-password")
            .await
            .unwrap();
        assert!(
            resolve_session(&pg, &keys, &current)
                .await
                .unwrap()
                .is_some()
        );
        assert!(resolve_session(&pg, &keys, &other).await.unwrap().is_none());
        let _: i64 = vk.del("login:fail:bob").await.unwrap();
        assert_eq!(login(&pg, &vk, "bob", "new-password").await.unwrap(), bob);
    }
}
