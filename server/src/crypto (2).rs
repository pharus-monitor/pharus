//! At-rest encryption for notification channel secrets.
//!
//! The master key lives in the `settings` table, so this protects secrets in
//! database dumps and backups — not against an attacker who already has read
//! access to the database file itself.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Result};
use rusqlite::Connection;
use serde_json::Value;

const PREFIX: &str = "enc:v1:";
const KEY_SETTING: &str = "secret_key";

/// Config keys whose values are encrypted at rest and redacted in API responses.
/// Webhook URLs are included because most providers embed the token in the path.
pub const SECRET_FIELDS: &[&str] = &[
    "token",
    "bot_token",
    "password",
    "secret",
    "device_key",
    "api_key",
    "url",
    "webhook_url",
];

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn cipher(conn: &Connection) -> Result<Aes256Gcm> {
    let stored = crate::db::get_setting(conn, KEY_SETTING)?;
    let key_bytes = match stored.as_deref().and_then(from_hex) {
        Some(b) if b.len() == 32 => b,
        _ => {
            let key = Aes256Gcm::generate_key(&mut OsRng);
            crate::db::set_setting(conn, KEY_SETTING, &to_hex(&key))?;
            key.to_vec()
        }
    };
    Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| anyhow!("bad master key: {e}"))
}

pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(PREFIX)
}

pub fn encrypt(conn: &Connection, plaintext: &str) -> Result<String> {
    let c = cipher(conn)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = c
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("encrypt failed: {e}"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ct);
    Ok(format!("{PREFIX}{}", to_hex(&blob)))
}

/// Values without the encryption prefix are returned unchanged, so configs
/// written before encryption was enabled keep working.
pub fn decrypt(conn: &Connection, stored: &str) -> Result<String> {
    let Some(hex) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let blob = from_hex(hex).ok_or_else(|| anyhow!("secret is not valid hex"))?;
    if blob.len() < 12 {
        return Err(anyhow!("secret too short"));
    }
    let (nonce, ct) = blob.split_at(12);
    let c = cipher(conn)?;
    let plain = c
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|e| anyhow!("decrypt failed: {e}"))?;
    Ok(String::from_utf8(plain)?)
}

/// Encrypt every secret-bearing field in a channel config before storing it.
pub fn seal(conn: &Connection, config: &mut Value) -> Result<()> {
    let Some(map) = config.as_object_mut() else {
        return Ok(());
    };
    for field in SECRET_FIELDS {
        let Some(Value::String(s)) = map.get(*field) else {
            continue;
        };
        if s.is_empty() || is_encrypted(s) {
            continue;
        }
        let sealed = encrypt(conn, s)?;
        map.insert((*field).to_string(), Value::String(sealed));
    }
    Ok(())
}

/// Decrypt a stored channel config for outbound use. Fields that fail to
/// decrypt are dropped rather than leaked as ciphertext.
pub fn reveal(conn: &Connection, config: &Value) -> Value {
    let mut out = config.clone();
    let Some(map) = out.as_object_mut() else {
        return out;
    };
    for field in SECRET_FIELDS {
        let Some(Value::String(s)) = map.get(*field) else {
            continue;
        };
        match decrypt(conn, s) {
            Ok(plain) => {
                map.insert((*field).to_string(), Value::String(plain));
            }
            Err(e) => {
                tracing::warn!(field, error = %e, "channel secret could not be decrypted");
                map.remove(*field);
            }
        }
    }
    out
}

/// Mask secret-bearing fields so a config can be returned over the admin API.
pub fn redact(config: &Value) -> Value {
    let mut out = config.clone();
    let Some(map) = out.as_object_mut() else {
        return out;
    };
    for field in SECRET_FIELDS {
        if let Some(Value::String(s)) = map.get(*field) {
            if !s.is_empty() {
                map.insert((*field).to_string(), Value::String("***".into()));
            }
        }
    }
    out
}
