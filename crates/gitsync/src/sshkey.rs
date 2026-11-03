//! SSH deploy keys and host keys (git-sync D19).

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use sha2::{Digest, Sha256};
use ssh_key::{Algorithm, LineEnding, PrivateKey};

use crate::{Error, Result};

/// A new ed25519 key pair: the OpenSSH private key and its public key line, both with
/// `comment`.
pub fn generate_ed25519(comment: &str) -> Result<(String, String)> {
    let mut rng = chacha20poly1305::aead::OsRng;
    let mut key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(|e| Error::SshKey(e.to_string()))?;
    key.set_comment(comment);
    let private = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| Error::SshKey(e.to_string()))?;
    let public = public_line(&key)?;
    Ok((private.to_string(), public))
}

/// The public key line of a pasted OpenSSH private key. A key protected by a passphrase is
/// refused, since a round has nobody to type it.
pub fn public_key_of(private_openssh: &str) -> Result<String> {
    let key = PrivateKey::from_openssh(private_openssh.trim())
        .map_err(|e| Error::SshKey(format!("not an OpenSSH private key: {e}")))?;
    if key.is_encrypted() {
        return Err(Error::SshKey(
            "the key is protected by a passphrase; paste a key without one".into(),
        ));
    }
    public_line(&key)
}

/// The SHA256 fingerprint (`SHA256:...`) of the host key in a `known_hosts` line,
/// `<hosts> <type> <base64> [comment]`.
pub fn host_fingerprint(known_hosts_line: &str) -> Result<String> {
    let mut fields = known_hosts_line.split_whitespace();
    let first = fields.next().unwrap_or_default();
    // A marker such as `@cert-authority` comes before the host list.
    if first.starts_with('@') {
        fields.next();
    }
    let (Some(kind), Some(data)) = (fields.next(), fields.next()) else {
        return Err(Error::SshKey("not a known_hosts line".into()));
    };
    // OpenSSH's fingerprint is the sha256 of the key blob, so any algorithm works without its
    // implementation.
    let blob = STANDARD
        .decode(data)
        .map_err(|_| Error::SshKey("the host key is not base64".into()))?;
    if !blob.starts_with(&ssh_string(kind)) {
        return Err(Error::SshKey(format!("the host key is not a {kind} key")));
    }
    Ok(format!(
        "SHA256:{}",
        STANDARD_NO_PAD.encode(Sha256::digest(&blob))
    ))
}

/// `text` as an SSH wire string: a big-endian u32 length, then the bytes.
fn ssh_string(text: &str) -> Vec<u8> {
    let mut out = (text.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(text.as_bytes());
    out
}

fn public_line(key: &PrivateKey) -> Result<String> {
    key.public_key()
        .to_openssh()
        .map_err(|e| Error::SshKey(e.to_string()))
}
