//! The shared key, and the blobs and names it makes (git-sync D4, D13, D14).
//!
//! The shared key is 32 random bytes, written as `tvsync1-` and base64url without padding. Three
//! subkeys come from it through HKDF-SHA256, under the labels `enc`, `nonce` and `name`.
//!
//! A blob is `TVGS`, a version byte, a 24-byte nonce, then the XChaCha20-Poly1305 ciphertext of
//! the plaintext, with the five header bytes as associated data. The nonce is the first 24 bytes
//! of HMAC-SHA256(nonce key, plaintext), so unchanged plaintext gives a byte-identical blob and
//! git stores it once. A blob that fails to authenticate is [`Error::KeyMismatch`].

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{Error, Result};

/// The prefix of a sync key's text form.
pub const KEY_PREFIX: &str = "tvsync1-";
/// The first bytes of every blob.
pub const MAGIC: &[u8; 4] = b"TVGS";
/// The blob version this build writes and reads.
pub const BLOB_VERSION: u8 = 1;
/// The bytes a blob adds to its plaintext: the header, the nonce and the Poly1305 tag.
pub const OVERHEAD: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;
/// The hex characters of a [`SyncKey::name`].
pub const NAME_LEN: usize = 32;

const KEY_LEN: usize = 32;
const HEADER_LEN: usize = 5;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

type HmacSha256 = Hmac<Sha256>;

/// The key every instance of one git-sync shares (D14).
#[derive(Clone, PartialEq, Eq)]
pub struct SyncKey {
    raw: [u8; KEY_LEN],
    enc: [u8; KEY_LEN],
    nonce: [u8; KEY_LEN],
    name: [u8; KEY_LEN],
}

impl std::fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncKey(..)")
    }
}

impl SyncKey {
    /// A new random key.
    pub fn generate() -> Self {
        use chacha20poly1305::aead::OsRng;
        use chacha20poly1305::aead::rand_core::RngCore;
        let mut raw = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut raw);
        Self::from_bytes(raw)
    }

    /// The key with these 32 bytes.
    pub fn from_bytes(raw: [u8; KEY_LEN]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, &raw);
        let derive = |label: &str| {
            let mut out = [0u8; KEY_LEN];
            hk.expand(label.as_bytes(), &mut out)
                .expect("32 bytes is a valid HKDF-SHA256 output length");
            out
        };
        Self {
            raw,
            enc: derive("enc"),
            nonce: derive("nonce"),
            name: derive("name"),
        }
    }

    /// Reads the `tvsync1-` text form. Whitespace around it is ignored.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let Some(body) = text.strip_prefix(KEY_PREFIX) else {
            return Err(Error::BadKey(format!("it must start with {KEY_PREFIX}")));
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| Error::BadKey("it is not base64url without padding".into()))?;
        let raw: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
            Error::BadKey(format!(
                "it holds {} bytes, expected {KEY_LEN}",
                bytes.len()
            ))
        })?;
        Ok(Self::from_bytes(raw))
    }

    /// The `tvsync1-` text form, the code the owner copies between instances.
    pub fn to_text(&self) -> String {
        format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(self.raw))
    }

    /// Encrypts `plaintext` into a blob. The same plaintext always gives the same blob.
    pub fn seal(&self, plaintext: &[u8]) -> Vec<u8> {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&self.nonce).expect("HMAC takes any key length");
        mac.update(plaintext);
        let tag = mac.finalize().into_bytes();
        let nonce = XNonce::from_slice(&tag[..NONCE_LEN]);
        let header = header();
        let ciphertext = self
            .cipher()
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: &header,
                },
            )
            .expect("XChaCha20-Poly1305 encrypts any length a blob holds");
        let mut blob = Vec::with_capacity(OVERHEAD + plaintext.len());
        blob.extend_from_slice(&header);
        blob.extend_from_slice(nonce);
        blob.extend_from_slice(&ciphertext);
        blob
    }

    /// Decrypts a blob [`SyncKey::seal`] made. A blob sealed with another key, or altered, is
    /// [`Error::KeyMismatch`]; one that is not a blob at all is [`Error::Malformed`].
    pub fn open(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < OVERHEAD || &blob[..4] != MAGIC {
            return Err(Error::Malformed("not a gitsync blob".into()));
        }
        if blob[4] != BLOB_VERSION {
            return Err(Error::Malformed(format!(
                "blob version {} is not {BLOB_VERSION}",
                blob[4]
            )));
        }
        let nonce = XNonce::from_slice(&blob[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
        self.cipher()
            .decrypt(
                nonce,
                Payload {
                    msg: &blob[HEADER_LEN + NONCE_LEN..],
                    aad: &blob[..HEADER_LEN],
                },
            )
            .map_err(|_| Error::KeyMismatch)
    }

    /// The keyed-hash name of `id` of kind `kind`: hex HMAC-SHA256(name key, `kind:id`), cut to
    /// [`NAME_LEN`] hex characters (D13). Kinds are `instance`, `note`, `asset`, `type` and
    /// `taxonomy`.
    pub fn name(&self, kind: &str, id: &str) -> String {
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&self.name).expect("HMAC takes any key length");
        mac.update(kind.as_bytes());
        mac.update(b":");
        mac.update(id.as_bytes());
        let digest = mac.finalize().into_bytes();
        let mut hex = to_hex(&digest);
        hex.truncate(NAME_LEN);
        hex
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.enc).into())
    }
}

fn header() -> [u8; HEADER_LEN] {
    [MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], BLOB_VERSION]
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}
