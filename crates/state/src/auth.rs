//! API tokens (D11), the one-time install codes that give an AI agent one, and the login
//! password (OQ-S3).
//!
//! An agent install code is stored as a hash, like a pairing code, and names the token it makes:
//! `agent-<name>` with the scope chosen when the code was issued. Redeeming it works once and
//! replaces any earlier token of that name, so installing an agent again with a new code rotates
//! its token.

use std::str::FromStr;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::devices::{code_hash, new_code};
use crate::{Error, Result, State, random_bytes, to_hex};

/// An agent install code as issued. The code itself is shown once; only its hash is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCode {
    /// Eight characters in two groups, such as `K7QM-3XRT`.
    pub code: String,
    /// The name of the token redeeming it makes, `agent-<name>`.
    pub token_name: String,
    pub scope: Scope,
    pub expires_ms: i64,
}

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

fn new_secret() -> String {
    let mut raw = [0u8; 32];
    random_bytes(&mut raw);
    format!("{TOKEN_PREFIX}{}", to_hex(&raw))
}

impl State {
    /// Creates a token and returns it with its secret. The secret is shown once; only its
    /// hash is stored.
    pub fn create_token(&self, name: &str, scope: Scope) -> Result<(TokenInfo, String)> {
        let secret = new_secret();
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

    /// Issue an agent install code that works once, until `ttl_ms` from now, and makes the token
    /// `agent-<name>` with `scope`. Expired codes are removed on the way.
    pub fn create_agent_code(&self, name: &str, scope: Scope, ttl_ms: i64) -> Result<AgentCode> {
        let code = new_code();
        let now = self.now();
        let expires_ms = now + ttl_ms;
        let conn = self.conn();
        conn.execute("DELETE FROM agent_codes WHERE expires_ms <= ?1", [now])?;
        conn.execute(
            "INSERT INTO agent_codes (code_hash, name, scope, created_ms, expires_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![code_hash(&code), name, scope.as_str(), now, expires_ms],
        )?;
        Ok(AgentCode {
            code,
            token_name: format!("agent-{name}"),
            scope,
            expires_ms,
        })
    }

    /// Use up an agent install code: create its token, replacing any token of the same name, and
    /// return it with its secret. `None` when the code is unknown, used or expired.
    pub fn redeem_agent_code(&self, code: &str) -> Result<Option<(TokenInfo, String)>> {
        let now = self.now();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let taken = tx
            .query_row(
                "DELETE FROM agent_codes WHERE code_hash = ?1 RETURNING name, scope, expires_ms",
                [code_hash(code)],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((name, scope, _)) = taken.filter(|(_, _, expires)| *expires > now) else {
            tx.commit()?;
            return Ok(None);
        };
        let scope: Scope = scope.parse()?;
        let name = format!("agent-{name}");
        let secret = new_secret();
        tx.execute("DELETE FROM tokens WHERE name = ?1", [&name])?;
        tx.execute(
            "INSERT INTO tokens (name, scope, secret_hash, created_ms) VALUES (?1, ?2, ?3, ?4)",
            params![name, scope.as_str(), token_hash(&secret), now],
        )?;
        tx.commit()?;
        let info = TokenInfo {
            name,
            scope,
            created_ms: now,
            last_used_ms: None,
        };
        Ok(Some((info, secret)))
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
