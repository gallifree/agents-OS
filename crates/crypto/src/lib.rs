//! Keys derived from the master key, sealed tokens, Relay global IDs and cache keys (see AGENTS.md).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, Generate, KeyInit, Payload},
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

/// Prefix byte of every sealed token. Bump it when keys rotate, and keep the old keys to open old tokens.
// shortcut: single key version; accept a list of previous master keys when rotation is needed.
const KEY_VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const APP: &str = "rust-agents";

/// Subkeys, one per purpose; the master key itself is dropped after derivation.
#[derive(Clone)]
pub struct Keys {
    ids: XChaCha20Poly1305,
    sessions: XChaCha20Poly1305,
    secrets: XChaCha20Poly1305,
    cache_key: [u8; 32],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("token expired")]
    Expired,
    #[error("invalid token")]
    Invalid,
}

impl Keys {
    /// `master_b64` is the base64 MASTER_KEY: 32 random bytes (`openssl rand -base64 32`).
    pub fn from_master(master_b64: &str) -> Result<Self> {
        let master = STANDARD
            .decode(master_b64.trim())
            .context("MASTER_KEY is not valid base64")?;
        if master.len() != 32 {
            bail!("MASTER_KEY must decode to 32 bytes, got {}", master.len());
        }
        let hk = Hkdf::<Sha256>::new(None, &master);
        let derive = |purpose: &str| {
            let mut okm = [0u8; 32];
            hk.expand(
                format!("{APP}/{purpose}/v{KEY_VERSION}").as_bytes(),
                &mut okm,
            )
            .expect("32 bytes is a valid HKDF-SHA256 output length");
            okm
        };
        let cipher = |purpose: &str| {
            XChaCha20Poly1305::new_from_slice(&derive(purpose)).expect("32-byte key")
        };
        Ok(Self {
            ids: cipher("ids"),
            sessions: cipher("sessions"),
            secrets: cipher("secrets"),
            cache_key: derive("cache-key"),
        })
    }

    // ---- Relay global IDs -------------------------------------------------------------

    /// Opaque, expiring ID for a GraphQL object (`gid://rust-agents/<Type>/<uuid>`).
    pub fn encode_id(&self, type_name: &str, id: Uuid, ttl: Duration) -> String {
        self.encode_gid("relay-id", type_name, id, ttl)
    }

    pub fn decode_id(&self, token: &str) -> Result<(String, Uuid), TokenError> {
        self.decode_gid("relay-id", token)
    }

    /// Like `encode_id`, for links handed to other people (share, invite, download): its own purpose and TTL.
    pub fn encode_capability(
        &self,
        purpose: &str,
        type_name: &str,
        id: Uuid,
        ttl: Duration,
    ) -> String {
        self.encode_gid(&format!("capability:{purpose}"), type_name, id, ttl)
    }

    pub fn decode_capability(
        &self,
        purpose: &str,
        token: &str,
    ) -> Result<(String, Uuid), TokenError> {
        self.decode_gid(&format!("capability:{purpose}"), token)
    }

    /// Stable, non-decodable client cache identity of an object (Relay `getDataID`).
    pub fn cache_key(&self, type_name: &str, id: Uuid) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.cache_key).expect("HMAC takes any key length");
        mac.update(format!("{type_name}:{id}").as_bytes());
        // 128 bits is plenty to avoid collisions between objects.
        URL_SAFE_NO_PAD.encode(&mac.finalize().into_bytes()[..16])
    }

    fn encode_gid(&self, purpose: &str, type_name: &str, id: Uuid, ttl: Duration) -> String {
        seal(
            &self.ids,
            purpose,
            Claims {
                gid: format!("gid://{APP}/{type_name}/{id}"),
            },
            ttl,
        )
    }

    fn decode_gid(&self, purpose: &str, token: &str) -> Result<(String, Uuid), TokenError> {
        let claims: Claims = open(&self.ids, purpose, token)?;
        let rest = claims
            .gid
            .strip_prefix(&format!("gid://{APP}/"))
            .ok_or(TokenError::Invalid)?;
        let (type_name, id) = rest.split_once('/').ok_or(TokenError::Invalid)?;
        Ok((
            type_name.into(),
            id.parse().map_err(|_| TokenError::Invalid)?,
        ))
    }

    // ---- Sessions -------------------------------------------------------------------------

    /// Session token: user and session ids, expiring. The session row in Postgres still decides revocation.
    pub fn encode_session(&self, session_id: Uuid, user_id: Uuid, ttl: Duration) -> String {
        seal(
            &self.sessions,
            "session",
            SessionClaims {
                sid: session_id,
                uid: user_id,
            },
            ttl,
        )
    }

    /// Returns `(session_id, user_id)`.
    pub fn decode_session(&self, token: &str) -> Result<(Uuid, Uuid), TokenError> {
        let c: SessionClaims = open(&self.sessions, "session", token)?;
        Ok((c.sid, c.uid))
    }

    // ---- Secrets at rest ------------------------------------------------------------------

    /// Encrypts a secret (provider API key…) for storage. `context` binds it to its owner, e.g. "provider:<uuid>",
    /// so a ciphertext copied to another row won't decrypt.
    pub fn encrypt_secret(&self, context: &str, secret: &[u8]) -> Vec<u8> {
        encrypt(&self.secrets, context, secret)
    }

    pub fn decrypt_secret(&self, context: &str, sealed: &[u8]) -> Result<Vec<u8>, TokenError> {
        decrypt(&self.secrets, context, sealed)
    }
}

#[derive(Serialize, Deserialize)]
struct Claims {
    gid: String,
}

#[derive(Serialize, Deserialize)]
struct SessionClaims {
    sid: Uuid,
    uid: Uuid,
}

/// Every sealed payload carries its expiry (unix seconds).
#[derive(Serialize, Deserialize)]
struct Expiring<T> {
    #[serde(flatten)]
    claims: T,
    exp: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_secs()
}

/// `version || nonce || ciphertext`; the purpose is authenticated data, so a token only opens for its purpose.
fn encrypt(cipher: &XChaCha20Poly1305, purpose: &str, msg: &[u8]) -> Vec<u8> {
    let nonce = XNonce::generate();
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg,
                aad: purpose.as_bytes(),
            },
        )
        .expect("XChaCha20-Poly1305 encryption does not fail for in-memory buffers");
    let mut out = Vec::with_capacity(1 + NONCE_LEN + ct.len());
    out.push(KEY_VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

fn decrypt(cipher: &XChaCha20Poly1305, purpose: &str, data: &[u8]) -> Result<Vec<u8>, TokenError> {
    let (&version, rest) = data.split_first().ok_or(TokenError::Invalid)?;
    if version != KEY_VERSION || rest.len() < NONCE_LEN {
        return Err(TokenError::Invalid);
    }
    let (nonce, ct) = rest.split_at(NONCE_LEN);
    let nonce = XNonce::try_from(nonce).map_err(|_| TokenError::Invalid)?;
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ct,
                aad: purpose.as_bytes(),
            },
        )
        .map_err(|_| TokenError::Invalid)
}

fn seal<T: Serialize>(
    cipher: &XChaCha20Poly1305,
    purpose: &str,
    claims: T,
    ttl: Duration,
) -> String {
    let json = serde_json::to_vec(&Expiring {
        claims,
        exp: now() + ttl.as_secs(),
    })
    .expect("claims serialize");
    URL_SAFE_NO_PAD.encode(encrypt(cipher, purpose, &json))
}

fn open<T: for<'de> Deserialize<'de>>(
    cipher: &XChaCha20Poly1305,
    purpose: &str,
    token: &str,
) -> Result<T, TokenError> {
    let data = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| TokenError::Invalid)?;
    let sealed: Expiring<T> = serde_json::from_slice(&decrypt(cipher, purpose, &data)?)
        .map_err(|_| TokenError::Invalid)?;
    if sealed.exp <= now() {
        return Err(TokenError::Expired);
    }
    Ok(sealed.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    fn keys() -> Keys {
        Keys::from_master(&STANDARD.encode([7u8; 32])).unwrap()
    }

    #[test]
    fn master_key_must_be_32_bytes_of_base64() {
        assert!(Keys::from_master("not base64!").is_err());
        assert!(Keys::from_master(&STANDARD.encode([0u8; 16])).is_err());
    }

    #[test]
    fn relay_id_round_trip_and_is_randomized() {
        let k = keys();
        let id = Uuid::now_v7();
        let a = k.encode_id("User", id, HOUR);
        let b = k.encode_id("User", id, HOUR);
        assert_ne!(a, b, "random nonce: same object, different tokens");
        assert_eq!(k.decode_id(&a), Ok(("User".into(), id)));
        assert!(!a.contains(&id.to_string()));
    }

    #[test]
    fn expired_and_tampered_ids_are_rejected() {
        let k = keys();
        let id = Uuid::now_v7();
        assert_eq!(
            k.decode_id(&k.encode_id("User", id, Duration::ZERO)),
            Err(TokenError::Expired)
        );

        let mut raw = URL_SAFE_NO_PAD
            .decode(k.encode_id("User", id, HOUR))
            .unwrap();
        *raw.last_mut().unwrap() ^= 1;
        assert_eq!(
            k.decode_id(&URL_SAFE_NO_PAD.encode(raw)),
            Err(TokenError::Invalid)
        );
        assert_eq!(k.decode_id("garbage"), Err(TokenError::Invalid));
    }

    #[test]
    fn tokens_only_open_for_their_purpose_and_key() {
        let k = keys();
        let id = Uuid::now_v7();
        let share = k.encode_capability("share", "File", id, HOUR);
        assert_eq!(
            k.decode_capability("share", &share),
            Ok(("File".into(), id))
        );
        assert_eq!(
            k.decode_capability("invite", &share),
            Err(TokenError::Invalid)
        );
        assert_eq!(k.decode_id(&share), Err(TokenError::Invalid));

        let other = Keys::from_master(&STANDARD.encode([8u8; 32])).unwrap();
        assert_eq!(
            other.decode_id(&k.encode_id("User", id, HOUR)),
            Err(TokenError::Invalid)
        );
    }

    #[test]
    fn cache_key_is_stable_per_object_and_distinct_across_types() {
        let k = keys();
        let id = Uuid::now_v7();
        assert_eq!(k.cache_key("User", id), k.cache_key("User", id));
        assert_ne!(k.cache_key("User", id), k.cache_key("Agent", id));
        assert_ne!(k.cache_key("User", id), keys_other().cache_key("User", id));
    }

    fn keys_other() -> Keys {
        Keys::from_master(&STANDARD.encode([9u8; 32])).unwrap()
    }

    #[test]
    fn sessions_round_trip_and_are_not_ids() {
        let k = keys();
        let (sid, uid) = (Uuid::now_v7(), Uuid::now_v7());
        let token = k.encode_session(sid, uid, HOUR);
        assert_eq!(k.decode_session(&token), Ok((sid, uid)));
        assert_eq!(k.decode_id(&token), Err(TokenError::Invalid));
        assert_eq!(
            k.decode_session(&k.encode_session(sid, uid, Duration::ZERO)),
            Err(TokenError::Expired)
        );
    }

    #[test]
    fn secrets_are_bound_to_their_context() {
        let k = keys();
        let sealed = k.encrypt_secret("provider:1", b"sk-test");
        assert_eq!(k.decrypt_secret("provider:1", &sealed).unwrap(), b"sk-test");
        assert_eq!(
            k.decrypt_secret("provider:2", &sealed),
            Err(TokenError::Invalid)
        );
    }
}
