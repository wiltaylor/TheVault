//! Secrets encrypted at rest with a key file kept outside the vault (D10, OQ-S2).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::{OptionalExtension, params};

use crate::{Error, Result, State, random_bytes};

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// Canonical HTTP(S) origin, shared by stored grants and browser page matching.
pub fn canonical_origin(origin: &str) -> Option<String> {
    let rest = origin.split_once("://")?.1;
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if origin.trim() != origin || authority.contains(['/', '\\', '@', '?', '#']) {
        return None;
    }
    let mut url = url::Url::parse(origin).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if let Some(url::Host::Domain(domain)) = url.host() {
        let domain = domain.strip_suffix('.').unwrap_or(domain);
        if domain.split('.').any(str::is_empty)
            || !domain
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'.'))
        {
            return None;
        }
        let domain = domain.to_owned();
        url.set_host(Some(&domain)).ok()?;
    }
    Some(url.origin().ascii_serialization())
}

/// A Login payload. Never format or serialize it into a response, log or script binding.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginSecret {
    pub username: String,
    pub password: String,
    pub totp_seed: Option<String>,
    pub origins: Vec<String>,
}

/// Safe Login metadata. Credentials deliberately have no fields here.
#[derive(Debug, Clone)]
pub struct LoginInfo {
    pub name: String,
    pub username: String,
    pub origins: Vec<String>,
    pub has_totp: bool,
    pub created_ms: i64,
    pub updated_ms: i64,
}

/// The 32-byte key that encrypts secrets in `state.db`.
#[derive(Clone)]
pub struct SecretKey([u8; KEY_LEN]);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

impl SecretKey {
    /// A new random key.
    pub fn generate() -> Self {
        let mut key = [0u8; KEY_LEN];
        random_bytes(&mut key);
        Self(key)
    }

    /// Generates a key and writes it to `path`, readable only by its owner. Fails if the
    /// file already exists, so an existing key is never overwritten.
    pub fn create_file(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let key = Self::generate();
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(&key.0)?;
        file.sync_all()?;
        Ok(key)
    }

    /// Reads a key file.
    pub fn load_file(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::KeyMissing {
                    path: Some(path.to_owned()),
                });
            }
            Err(e) => return Err(e.into()),
        };
        let key: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| Error::WrongKey {
            detail: format!(
                "{} holds {} bytes, expected {KEY_LEN}",
                path.display(),
                bytes.len()
            ),
        })?;
        Ok(Self(key))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.0).into())
    }
}

/// Where a vault's key file lives: `VAULT_SECRET_KEY_FILE` when set, otherwise
/// `$XDG_CONFIG_HOME/thevault/<vault-id>.key`, falling back to `$HOME/.config`. Takes the
/// environment values as arguments; pass `std::env::var_os` results.
pub fn key_file_path(
    vault_id: &str,
    override_path: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let set = |p: Option<PathBuf>| p.filter(|p| !p.as_os_str().is_empty());
    if let Some(path) = set(override_path) {
        return Some(path);
    }
    let config = set(xdg_config_home).or_else(|| set(home).map(|h| h.join(".config")))?;
    Some(config.join("thevault").join(format!("{vault_id}.key")))
}

/// A stored secret's metadata. The value is never listed (D10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretInfo {
    pub name: String,
    pub created_ms: i64,
    pub updated_ms: i64,
}

impl State {
    /// Stores a Login in a separate, encrypted namespace, never readable by `get_secret`.
    pub fn set_login_secret(&self, name: &str, login: &LoginSecret) -> Result<()> {
        // Replacing must not silently overwrite a credential under the wrong key.
        self.get_login_secret(name)?;
        let payload = serde_json::to_vec(login)
            .map_err(|_| Error::Corrupt("login encoding failed".into()))?;
        let mut nonce = [0u8; NONCE_LEN];
        random_bytes(&mut nonce);
        let aad = format!("login:{name}");
        let ciphertext = self
            .key()?
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &payload,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::Corrupt("login encryption failed".into()))?;
        let origins = serde_json::to_string(&login.origins)
            .map_err(|_| Error::Corrupt("login origins encoding failed".into()))?;
        self.conn().execute("INSERT INTO login_secrets (name,nonce,ciphertext,created_ms,updated_ms,username,origins,has_totp) VALUES (?1,?2,?3,?4,?4,?5,?6,?7) ON CONFLICT(name) DO UPDATE SET nonce=excluded.nonce,ciphertext=excluded.ciphertext,updated_ms=excluded.updated_ms,username=excluded.username,origins=excluded.origins,has_totp=excluded.has_totp",
            params![name, nonce.as_slice(), ciphertext, self.now(), login.username, origins, login.totp_seed.is_some()])?;
        Ok(())
    }

    /// Server-only reader for browser filling. Not exposed to string-secret consumers.
    pub fn get_login_secret(&self, name: &str) -> Result<Option<LoginSecret>> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .conn()
            .query_row(
                "SELECT nonce,ciphertext FROM login_secrets WHERE name=?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((nonce, ciphertext)) = row else {
            return Ok(None);
        };
        if nonce.len() != NONCE_LEN {
            return Err(Error::Corrupt("malformed login nonce".into()));
        }
        let aad = format!("login:{name}");
        let plaintext = self
            .key()?
            .cipher()
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::WrongKey {
                detail: format!("login {name:?} could not be decrypted with the loaded key file"),
            })?;
        serde_json::from_slice(&plaintext)
            .map(Some)
            .map_err(|_| Error::Corrupt("invalid login payload".into()))
    }

    pub fn list_login_secrets(&self) -> Result<Vec<LoginInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT name,created_ms,updated_ms,username,origins,has_totp FROM login_secrets ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            let origins: String = r.get(4)?;
            Ok(LoginInfo {
                name: r.get(0)?,
                created_ms: r.get(1)?,
                updated_ms: r.get(2)?,
                username: r.get(3)?,
                origins: serde_json::from_str(&origins).unwrap_or_default(),
                has_totp: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn is_login_secret(&self, name: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM login_secrets WHERE name=?1)",
            [name],
            |r| r.get(0),
        )?)
    }

    pub fn delete_login_secret(&self, name: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if tx.execute("DELETE FROM login_secrets WHERE name=?1", [name])? == 0 {
            return Err(Error::UnknownSecret(name.to_owned()));
        }
        tx.execute("INSERT INTO browser_secret_events(kind,name,time_ms) VALUES ('deleted',?1,MAX(?2,COALESCE((SELECT MAX(time_ms)+1 FROM browser_secret_events),?2))) ON CONFLICT(kind,name) DO UPDATE SET time_ms=excluded.time_ms", params![name, self.now()])?;
        tx.commit()?;
        Ok(())
    }

    /// Called only after an owner saves an agent through the admin definition route.
    /// The logical millisecond clock also orders events when wall time stands still or rolls back.
    pub fn confirm_browser_secrets(&self, agent: &str, names: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO browser_secret_events(kind,name,time_ms) VALUES ('saved',?1,MAX(?2,COALESCE((SELECT MAX(time_ms)+1 FROM browser_secret_events),?2))) ON CONFLICT(kind,name) DO UPDATE SET time_ms=excluded.time_ms", params![agent, self.now()])?;
        tx.execute("DELETE FROM browser_secret_grants WHERE agent=?1", [agent])?;
        for name in names {
            tx.execute("INSERT INTO browser_secret_grants(agent,name,time_ms) SELECT ?1,?2,time_ms FROM browser_secret_events WHERE kind='saved' AND name=?1", params![agent, name])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Revokes all confirmed names when an agent is deleted, including proposal Apply.
    pub fn revoke_browser_secrets(&self, agent: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM browser_secret_grants WHERE agent=?1", [agent])?;
        Ok(())
    }

    /// A proposal may remove confirmations, never refresh or add them.
    pub fn narrow_browser_secrets(&self, agent: &str, names: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let previous: Vec<String> = {
            let mut stmt = tx.prepare("SELECT name FROM browser_secret_grants WHERE agent=?1")?;
            stmt.query_map([agent], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for name in previous {
            if !names.contains(&name) {
                tx.execute(
                    "DELETE FROM browser_secret_grants WHERE agent=?1 AND name=?2",
                    params![agent, name],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Revoke grants for agent files whose absence the writer has observed.
    pub fn revoke_missing_browser_secrets(&self, exists: impl Fn(&str) -> bool) -> Result<()> {
        let conn = self.conn();
        let agents: Vec<String> = {
            let mut stmt = conn.prepare("SELECT DISTINCT agent FROM browser_secret_grants")?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for agent in agents {
            if !exists(&agent) {
                conn.execute("DELETE FROM browser_secret_grants WHERE agent=?1", [agent])?;
            }
        }
        Ok(())
    }

    /// Liveness only, not an authorization check: callers must also check the agent's list.
    /// Missing save records fail closed, including files imported without owner confirmation.
    pub fn browser_secret_active(&self, agent: &str, name: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT EXISTS(SELECT 1 FROM login_secrets l JOIN browser_secret_grants g ON g.agent=?1 AND g.name=l.name WHERE l.name=?2 AND g.time_ms > COALESCE((SELECT time_ms FROM browser_secret_events WHERE kind='deleted' AND name=?2),-1))", params![agent, name], |r| r.get(0))?)
    }

    /// Whether a key file was loaded, so secrets can be read and written.
    pub fn has_secret_key(&self) -> bool {
        self.key.is_some()
    }

    fn key(&self) -> Result<&SecretKey> {
        self.key.as_ref().ok_or(Error::KeyMissing { path: None })
    }

    /// Stores or replaces a secret, encrypted under the key file.
    pub fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        let cipher = self.key()?.cipher();
        let mut nonce = [0u8; NONCE_LEN];
        random_bytes(&mut nonce);
        // The name is bound in as associated data, so ciphertexts cannot be swapped between rows.
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: value.as_bytes(),
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| Error::Corrupt("secret encryption failed".into()))?;
        let now = self.now();
        self.conn().execute(
            "INSERT INTO secrets (name, nonce, ciphertext, created_ms, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT (name) DO UPDATE SET
                 nonce = excluded.nonce, ciphertext = excluded.ciphertext,
                 updated_ms = excluded.updated_ms",
            params![name, nonce.as_slice(), ciphertext, now],
        )?;
        Ok(())
    }

    /// Reads a secret by name. `None` when no secret has that name.
    pub fn get_secret(&self, name: &str) -> Result<Option<String>> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .conn()
            .query_row(
                "SELECT nonce, ciphertext FROM secrets WHERE name = ?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((nonce, ciphertext)) = row else {
            return Ok(None);
        };
        let cipher = self.key()?.cipher();
        if nonce.len() != NONCE_LEN {
            return Err(Error::Corrupt(format!(
                "secret {name:?} has a malformed nonce"
            )));
        }
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| Error::WrongKey {
                detail: format!("secret {name:?} could not be decrypted with the loaded key file"),
            })?;
        String::from_utf8(plaintext)
            .map(Some)
            .map_err(|_| Error::Corrupt(format!("secret {name:?} is not UTF-8")))
    }

    /// Every secret's name and timestamps, by name. Needs no key.
    pub fn list_secrets(&self) -> Result<Vec<SecretInfo>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT name, created_ms, updated_ms FROM secrets ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(SecretInfo {
                name: r.get(0)?,
                created_ms: r.get(1)?,
                updated_ms: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Deletes a secret.
    pub fn delete_secret(&self, name: &str) -> Result<()> {
        match self
            .conn()
            .execute("DELETE FROM secrets WHERE name = ?1", [name])?
        {
            0 => Err(Error::UnknownSecret(name.to_owned())),
            _ => Ok(()),
        }
    }
}
