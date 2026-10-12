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
