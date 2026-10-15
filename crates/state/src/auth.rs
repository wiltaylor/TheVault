//! API tokens (D11) and the login password (OQ-S3).

use std::str::FromStr;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{Error, Result, State, random_bytes, to_hex};

/// What a token may do. Each scope includes the ones below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Reads only.
    Read,
    /// Note and asset changes.
    Write,
    /// Everything, including types, templates, taxonomies, scripts, tasks and secrets.
    Admin,
}

impl Scope {
    /// Whether a token with this scope may do something that needs `required`.
    pub fn allows(self, required: Scope) -> bool {
        self >= required
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
            Scope::Admin => "admin",
        }
    }
}

impl FromStr for Scope {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "read" => Ok(Scope::Read),
            "write" => Ok(Scope::Write),
            "admin" => Ok(Scope::Admin),
            other => Err(Error::UnknownScope(other.to_owned())),
        }
    }
}

/// A token as stored, without its secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenInfo {
    pub name: String,
    pub scope: Scope,
    pub created_ms: i64,
    pub last_used_ms: Option<i64>,
}

/// Prefix of every token secret, so a leaked one is recognisable.
const TOKEN_PREFIX: &str = "tv_";

/// Token secrets carry 256 random bits, so a plain SHA-256 is enough to store them: there is
/// nothing to brute-force, and lookups by hash stay fast on every request.
fn token_hash(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

fn token_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(TokenInfo, String)> {
    Ok((
        TokenInfo {
            name: row.get(0)?,
            scope: Scope::Read,
            created_ms: row.get(2)?,
            last_used_ms: row.get(3)?,
        },
        row.get(1)?,
    ))
}

fn finish_token((mut info, scope): (TokenInfo, String)) -> Result<TokenInfo> {
    info.scope = scope
        .parse()
        .map_err(|_| Error::Corrupt(format!("token {:?} has scope {scope:?}", info.name)))?;
    Ok(info)
}

impl State {
    /// Creates a token and returns it with its secret. The secret is shown once; only its
    /// hash is stored.
    pub fn create_token(&self, name: &str, scope: Scope) -> Result<(TokenInfo, String)> {
        let mut raw = [0u8; 32];
        random_bytes(&mut raw);
        let secret = format!("{TOKEN_PREFIX}{}", to_hex(&raw));
        let created_ms = self.now();
        let inserted = self.conn().execute(
            "INSERT INTO tokens (name, scope, secret_hash, created_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (name) DO NOTHING",
            params![name, scope.as_str(), token_hash(&secret), created_ms],
        )?;
        if inserted == 0 {
            return Err(Error::TokenExists(name.to_owned()));
        }
        let info = TokenInfo {
            name: name.to_owned(),
            scope,
            created_ms,
            last_used_ms: None,
        };
        Ok((info, secret))
    }

    /// Finds the token a secret belongs to and records that it was used.
    pub fn authenticate_token(&self, secret: &str) -> Result<Option<TokenInfo>> {
        let now = self.now();
        let conn = self.conn();
        let row = conn
            .query_row(
                "UPDATE tokens SET last_used_ms = ?1 WHERE secret_hash = ?2
                 RETURNING name, scope, created_ms, last_used_ms",
                params![now, token_hash(secret)],
                token_row,
            )
            .optional()?;
        if let Some((info, _)) = &row {
            conn.execute(
                "UPDATE devices SET last_seen_ms = ?1 WHERE token_id = ?2",
                params![now, info.name],
            )?;
        }
        row.map(finish_token).transpose()
    }

    /// Every token, by name.
    pub fn list_tokens(&self) -> Result<Vec<TokenInfo>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT name, scope, created_ms, last_used_ms FROM tokens ORDER BY name")?;
        let rows = stmt.query_map([], token_row)?;
        rows.map(|r| finish_token(r?)).collect()
    }

    /// Deletes a token, so its secret stops working.
    pub fn revoke_token(&self, name: &str) -> Result<()> {
        match self
            .conn()
            .execute("DELETE FROM tokens WHERE name = ?1", [name])?
        {
            0 => Err(Error::UnknownToken(name.to_owned())),
            _ => Ok(()),
        }
    }

    /// Sets or replaces the login password, stored as an Argon2id hash.
    pub fn set_password(&self, password: &str) -> Result<()> {
        let mut raw = [0u8; 16];
        random_bytes(&mut raw);
        let salt = SaltString::encode_b64(&raw).map_err(|e| Error::PasswordHash(e.to_string()))?;
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| Error::PasswordHash(e.to_string()))?
            .to_string();
        self.conn().execute(
            "INSERT INTO password (id, hash) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET hash = excluded.hash",
            [hash],
        )?;
        Ok(())
    }

    /// Whether a login password has been set.
    pub fn has_password(&self) -> Result<bool> {
        Ok(self.stored_password_hash()?.is_some())
    }

    /// Checks a login attempt. Always false while no password is set.
    pub fn verify_password(&self, password: &str) -> Result<bool> {
        let Some(stored) = self.stored_password_hash()? else {
            return Ok(false);
        };
        let hash = PasswordHash::new(&stored)
            .map_err(|e| Error::Corrupt(format!("password hash: {e}")))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok())
    }

    fn stored_password_hash(&self) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT hash FROM password WHERE id = 1", [], |r| r.get(0))
            .optional()?)
    }
}
