//! WebPush message encryption (RFC 8291): one `aes128gcm` record (RFC 8188) encrypted to a
//! subscription's P-256 public key and authentication secret.
//!
//! [`encrypt`] is what the server sends. [`decrypt`] is the receiving side, for a device written
//! in Rust and for tests.

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{Aes128Gcm, KeyInit};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use p256::ecdh::diffie_hellman;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use sha2::Sha256;

/// The record size written into every message header. A message is one record, so this bounds
/// the plaintext.
pub const RECORD_SIZE: u32 = 4096;

/// The AES-GCM tag and the padding delimiter, which every record carries besides its plaintext.
const RECORD_OVERHEAD: usize = 16 + 1;

/// Why a message could not be encrypted or decrypted.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("{0} is not valid")]
    BadKey(&'static str),
    #[error("the plaintext does not fit one {RECORD_SIZE}-byte record")]
    TooLong,
    #[error("the message is malformed: {0}")]
    Malformed(&'static str),
    #[error("the message does not decrypt with these keys")]
    Decrypt,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Decode a base64url key, with or without padding.
pub fn decode_key(value: &str, what: &'static str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .map_err(|_| Error::BadKey(what))
}

/// Encrypt `plaintext` to the subscription whose public key is `p256dh` and authentication
/// secret is `auth`, both base64url, with a fresh sender key and salt.
pub fn encrypt(p256dh: &str, auth: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    let ua_public = decode_key(p256dh, "p256dh")?;
    let auth = decode_key(auth, "auth")?;
    let as_secret = SecretKey::random(&mut OsRng);
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    encrypt_with(&ua_public, &auth, &as_secret, &salt, plaintext)
}

/// [`encrypt`] with the sender key and salt given, so a known answer can be checked.
pub fn encrypt_with(
    ua_public: &[u8],
    auth: &[u8],
    as_secret: &SecretKey,
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    if auth.len() != 16 {
        return Err(Error::BadKey("auth"));
    }
    if plaintext.len() + RECORD_OVERHEAD > RECORD_SIZE as usize {
        return Err(Error::TooLong);
    }
    let ua = PublicKey::from_sec1_bytes(ua_public).map_err(|_| Error::BadKey("p256dh"))?;
    let ua_public = ua.to_encoded_point(false);
    let as_public = as_secret.public_key().to_encoded_point(false);
    let shared = diffie_hellman(as_secret.to_nonzero_scalar(), ua.as_affine());
    let (cek, nonce) = keys(
        shared.raw_secret_bytes(),
        auth,
        ua_public.as_bytes(),
        as_public.as_bytes(),
        salt,
    );
    let mut record = plaintext.to_vec();
    record.push(2);
    let sealed = Aes128Gcm::new(&cek.into())
        .encrypt(&nonce.into(), record.as_slice())
        .map_err(|_| Error::Decrypt)?;
    let key_id = as_public.as_bytes();
    let mut out = Vec::with_capacity(21 + key_id.len() + sealed.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(key_id.len() as u8);
    out.extend_from_slice(key_id);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Decrypt a message sent to the subscription whose private key is `ua_secret` and
/// authentication secret is `auth`. Only single-record messages, which is all [`encrypt`] makes.
pub fn decrypt(ua_secret: &SecretKey, auth: &[u8], message: &[u8]) -> Result<Vec<u8>> {
    if message.len() < 21 {
        return Err(Error::Malformed("shorter than its header"));
    }
    let salt: [u8; 16] = message[..16].try_into().expect("16 bytes");
    let rs = u32::from_be_bytes(message[16..20].try_into().expect("4 bytes")) as usize;
    let id_len = usize::from(message[20]);
    let Some(key_id) = message.get(21..21 + id_len) else {
        return Err(Error::Malformed("shorter than its key id"));
    };
    let sealed = &message[21 + id_len..];
    if sealed.len() > rs || sealed.len() < RECORD_OVERHEAD {
        return Err(Error::Malformed("not one record"));
    }
    let as_public = PublicKey::from_sec1_bytes(key_id).map_err(|_| Error::BadKey("key id"))?;
    let ua_public = ua_secret.public_key().to_encoded_point(false);
    let shared = diffie_hellman(ua_secret.to_nonzero_scalar(), as_public.as_affine());
    let (cek, nonce) = keys(
        shared.raw_secret_bytes(),
        auth,
        ua_public.as_bytes(),
        key_id,
        &salt,
    );
    let mut record = Aes128Gcm::new(&cek.into())
        .decrypt(&nonce.into(), sealed)
        .map_err(|_| Error::Decrypt)?;
    // Padding is zeros after a 2, the delimiter of the last record.
    while record.last() == Some(&0) {
        record.pop();
    }
    if record.pop() != Some(2) {
        return Err(Error::Malformed("no last-record delimiter"));
    }
    Ok(record)
}

/// The content encryption key and nonce (RFC 8291 section 3.4, RFC 8188 section 2.2).
fn keys(
    ecdh_secret: &[u8],
    auth: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8],
) -> ([u8; 16], [u8; 12]) {
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(ua_public);
    info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), ecdh_secret)
        .expand(&info, &mut ikm)
        .expect("32 bytes is a valid length");
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .expect("16 bytes is a valid length");
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .expect("12 bytes is a valid length");
    (cek, nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b64: &str) -> Vec<u8> {
        decode_key(b64, "test key").unwrap()
    }

    /// RFC 8291 appendix A.
    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const MESSAGE: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    #[test]
    fn the_rfc_8291_example_encrypts_to_its_published_message() {
        let as_secret = SecretKey::from_slice(&key(AS_PRIVATE)).unwrap();
        let salt: [u8; 16] = key(SALT).try_into().unwrap();
        let out = encrypt_with(
            &key(UA_PUBLIC),
            &key(AUTH),
            &as_secret,
            &salt,
            PLAINTEXT.as_bytes(),
        )
        .unwrap();
        assert_eq!(URL_SAFE_NO_PAD.encode(out), MESSAGE);
    }

    #[test]
    fn the_rfc_8291_example_decrypts() {
        let ua_secret = SecretKey::from_slice(&key(UA_PRIVATE)).unwrap();
        let out = decrypt(&ua_secret, &key(AUTH), &key(MESSAGE)).unwrap();
        assert_eq!(out, PLAINTEXT.as_bytes());
    }

    #[test]
    fn a_fresh_message_decrypts_only_with_the_right_keys() {
        let ua_secret = SecretKey::random(&mut OsRng);
        let ua_public = URL_SAFE_NO_PAD.encode(ua_secret.public_key().to_encoded_point(false));
        let auth = [7u8; 16];
        let sent = encrypt(&ua_public, &URL_SAFE_NO_PAD.encode(auth), b"{\"id\":\"x\"}").unwrap();
        assert_eq!(
            decrypt(&ua_secret, &auth, &sent).unwrap(),
            b"{\"id\":\"x\"}"
        );
        assert_eq!(decrypt(&ua_secret, &[8u8; 16], &sent), Err(Error::Decrypt));
        let other = SecretKey::random(&mut OsRng);
        assert_eq!(decrypt(&other, &auth, &sent), Err(Error::Decrypt));
        // Two messages with the same content differ: fresh key and salt each time.
        let again = encrypt(&ua_public, &URL_SAFE_NO_PAD.encode(auth), b"{\"id\":\"x\"}").unwrap();
        assert_ne!(sent, again);
    }

    #[test]
    fn bad_keys_and_oversized_plaintext_are_refused() {
        assert_eq!(
            encrypt("not base64!", AUTH, b"x"),
            Err(Error::BadKey("p256dh"))
        );
        assert_eq!(
            encrypt(UA_PUBLIC, "c2hvcnQ", b"x"),
            Err(Error::BadKey("auth"))
        );
        assert_eq!(encrypt("c2hvcnQ", AUTH, b"x"), Err(Error::BadKey("p256dh")));
        let big = vec![b'a'; RECORD_SIZE as usize];
        assert_eq!(encrypt(UA_PUBLIC, AUTH, &big), Err(Error::TooLong));
    }
}
