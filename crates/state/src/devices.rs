//! Paired devices and their pairing codes (D2, D3).
//!
//! A device pairs once and gets a `write` token of its own, named `device-<id>`, that only it
//! holds. The device row points at that token, and deleting the token deletes the row, so revoking
//! a device through `/api/devices` or through `/api/tokens` ends the same thing. A token's
//! successful use is the device's last-seen time.
//!
//! `capabilities` and `settings` are JSON text that this crate stores and never reads; the API
//! checks their shape (D25). A pairing code is stored as a hash, works once and expires.

use rusqlite::{OptionalExtension, Row, params};
use sha2::{Digest, Sha256};

use crate::{Error, Result, Scope, State, random_bytes};

/// The platforms a device can be.
pub const PLATFORMS: [&str; 2] = ["linux", "android"];

/// Where push messages for a device go: its UnifiedPush endpoint and the WebPush keys the
/// messages are encrypted to (RFC 8291).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTarget {
    pub endpoint: String,
    /// The device's P-256 public key, base64url.
    pub p256dh: String,
    /// The device's authentication secret, base64url.
    pub auth: String,
}

/// A paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// A ULID.
    pub id: String,
    pub name: String,
    /// One of [`PLATFORMS`].
    pub platform: String,
    /// JSON text.
    pub capabilities: String,
    /// The name of the device's token.
    pub token_id: String,
    pub push: Option<PushTarget>,
    pub last_seen_ms: Option<i64>,
    /// JSON text.
    pub settings: String,
    pub created_ms: i64,
}

/// What a new device is paired with.
#[derive(Debug, Clone)]
pub struct NewDevice<'a> {
    pub name: &'a str,
    pub platform: &'a str,
    pub capabilities: &'a str,
    pub settings: &'a str,
}

/// A pairing code as issued. The code itself is shown once; only its hash is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairCode {
    /// Eight characters in two groups, such as `K7QM-3XRT`.
    pub code: String,
    pub expires_ms: i64,
}

/// Crockford's base32 alphabet: no I, L, O or U, so a code read aloud or typed is hard to get wrong.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A code the way it is stored: upper case, with the hyphen, spaces and look-alike letters folded.
fn normalise_code(code: &str) -> String {
    code.chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        })
        .collect()
}

fn code_hash(code: &str) -> Vec<u8> {
    Sha256::digest(normalise_code(code).as_bytes()).to_vec()
}

const DEVICE_COLUMNS: &str = "id, name, platform, capabilities, token_id, push_endpoint, \
     push_p256dh, push_auth, last_seen_ms, settings, created_ms";

fn device_row(row: &Row<'_>) -> rusqlite::Result<Device> {
    let endpoint: Option<String> = row.get(5)?;
    let p256dh: Option<String> = row.get(6)?;
    let auth: Option<String> = row.get(7)?;
    let push = match (endpoint, p256dh, auth) {
        (Some(endpoint), Some(p256dh), Some(auth)) => Some(PushTarget {
            endpoint,
            p256dh,
            auth,
        }),
        _ => None,
    };
    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        platform: row.get(2)?,
        capabilities: row.get(3)?,
        token_id: row.get(4)?,
        push,
        last_seen_ms: row.get(8)?,
        settings: row.get(9)?,
        created_ms: row.get(10)?,
    })
}

impl State {
    /// Issue a pairing code that works once, until `ttl_ms` from now. Expired codes are removed
    /// on the way.
    pub fn create_pair_code(&self, ttl_ms: i64) -> Result<PairCode> {
        let mut raw = [0u8; 8];
        random_bytes(&mut raw);
        let chars: String = raw
            .iter()
            .map(|b| CODE_ALPHABET[usize::from(b % 32)] as char)
            .collect();
        let code = format!("{}-{}", &chars[..4], &chars[4..]);
        let now = self.now();
        let expires_ms = now + ttl_ms;
        let conn = self.conn();
        conn.execute("DELETE FROM pair_codes WHERE expires_ms <= ?1", [now])?;
        conn.execute(
            "INSERT INTO pair_codes (code_hash, created_ms, expires_ms) VALUES (?1, ?2, ?3)",
            params![code_hash(&code), now, expires_ms],
        )?;
        Ok(PairCode { code, expires_ms })
    }

    /// Use up a pairing code. False when it is unknown, used or expired.
    pub fn take_pair_code(&self, code: &str) -> Result<bool> {
        let now = self.now();
        let conn = self.conn();
        let taken = conn
            .query_row(
                "DELETE FROM pair_codes WHERE code_hash = ?1 RETURNING expires_ms",
                [code_hash(code)],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok(taken.is_some_and(|expires| expires > now))
    }

    /// Record a new device with a `write` token bound to it, and return the device with the
    /// token's secret. The secret is shown once; only its hash is stored.
    pub fn pair_device(&self, new: &NewDevice<'_>) -> Result<(Device, String)> {
        if !PLATFORMS.contains(&new.platform) {
            return Err(Error::UnknownPlatform(new.platform.to_owned()));
        }
        let id = self.new_ulid().to_string();
        let token_id = format!("device-{}", id.to_ascii_lowercase());
        let (token, secret) = self.create_token(&token_id, Scope::Write)?;
        let inserted = self.conn().execute(
            "INSERT INTO devices (id, name, platform, capabilities, token_id, settings, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                new.name,
                new.platform,
                new.capabilities,
                token_id,
                new.settings,
                token.created_ms
            ],
        );
        if let Err(e) = inserted {
            let _ = self.revoke_token(&token_id);
            return Err(e.into());
        }
        Ok((self.device(&id)?, secret))
    }

    /// Every device, oldest first.
    pub fn list_devices(&self) -> Result<Vec<Device>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {DEVICE_COLUMNS} FROM devices ORDER BY created_ms, id"
        ))?;
        let rows = stmt.query_map([], device_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One device.
    pub fn device(&self, id: &str) -> Result<Device> {
        self.conn()
            .query_row(
                &format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE id = ?1"),
                [id],
                device_row,
            )
            .optional()?
            .ok_or_else(|| Error::UnknownDevice(id.to_owned()))
    }

    /// The device a token belongs to, if it is a device token.
    pub fn device_for_token(&self, token_id: &str) -> Result<Option<Device>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {DEVICE_COLUMNS} FROM devices WHERE token_id = ?1"),
                [token_id],
                device_row,
            )
            .optional()?)
    }

    /// Change a device's name, its settings, or both.
    pub fn update_device(
        &self,
        id: &str,
        name: Option<&str>,
        settings: Option<&str>,
    ) -> Result<Device> {
        let changed = self.conn().execute(
            "UPDATE devices SET name = COALESCE(?2, name), settings = COALESCE(?3, settings)
             WHERE id = ?1",
            params![id, name, settings],
        )?;
        if changed == 0 {
            return Err(Error::UnknownDevice(id.to_owned()));
        }
        self.device(id)
    }

    /// Register where the device's push messages go, or clear it with `None`.
    pub fn set_device_push(&self, id: &str, push: Option<&PushTarget>) -> Result<Device> {
        let changed = self.conn().execute(
            "UPDATE devices SET push_endpoint = ?2, push_p256dh = ?3, push_auth = ?4 WHERE id = ?1",
            params![
                id,
                push.map(|p| &p.endpoint),
                push.map(|p| &p.p256dh),
                push.map(|p| &p.auth)
            ],
        )?;
        if changed == 0 {
            return Err(Error::UnknownDevice(id.to_owned()));
        }
        self.device(id)
    }

    /// Forget the device's push endpoint because the push service said it is gone, but only if
    /// it is still `endpoint`: a device that registered a new one meanwhile keeps it. True when
    /// it was removed.
    pub fn drop_device_push(&self, id: &str, endpoint: &str) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE devices SET push_endpoint = NULL, push_p256dh = NULL, push_auth = NULL
             WHERE id = ?1 AND push_endpoint = ?2",
            params![id, endpoint],
        )?;
        Ok(changed > 0)
    }

    /// Revoke a device: delete its token, which deletes the device with its push endpoint.
    pub fn revoke_device(&self, id: &str) -> Result<()> {
        let device = self.device(id)?;
        self.revoke_token(&device.token_id)
    }
}
