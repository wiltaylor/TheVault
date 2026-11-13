//! Notes and assets as blobs (git-sync D12).
//!
//! A note blob seals the note's type name, a newline, then its canonical WCL bytes. An asset is
//! split into parts whose blobs are each at most the part size, and reassembled by checking its
//! size and sha256.

use sha2::{Digest, Sha256};

use crate::crypto::{OVERHEAD, SyncKey, to_hex};
use crate::layout::Layout;
use crate::{Error, Result};

/// The sha256 of `bytes`, in lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

/// The plaintext of a note blob: `<type>\n<wcl>`.
pub fn note_plaintext(note_type: &str, wcl: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(note_type.len() + 1 + wcl.len());
    out.extend_from_slice(note_type.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(wcl);
    out
}

/// Splits a note blob's plaintext into its type name and WCL bytes.
pub fn parse_note(plaintext: &[u8]) -> Result<(String, Vec<u8>)> {
    let nl = plaintext
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| Error::Malformed("note blob has no type line".into()))?;
    let note_type = std::str::from_utf8(&plaintext[..nl])
        .map_err(|_| Error::Malformed("note type is not UTF-8".into()))?;
    Ok((note_type.to_owned(), plaintext[nl + 1..].to_vec()))
}

/// Seals a note into its blob.
pub fn seal_note(key: &SyncKey, path: &str, note_type: &str, wcl: &[u8]) -> Vec<u8> {
    key.seal(path, &note_plaintext(note_type, wcl))
}

/// Opens a note blob into its type name and WCL bytes.
pub fn open_note(key: &SyncKey, path: &str, blob: &[u8]) -> Result<(String, Vec<u8>)> {
    parse_note(&key.open(path, blob)?)
}

/// Splits `bytes` into part blobs of at most `part_bytes` each, in order. An empty asset is one
/// empty part. `part_bytes` must exceed a blob's overhead.
pub fn split(key: &SyncKey, layout: &Layout<'_>, id: &str, bytes: &[u8], part_bytes: usize) -> Vec<Vec<u8>> {
    assert!(
        part_bytes > OVERHEAD,
        "a part must hold more than the {OVERHEAD} bytes of a blob's overhead"
    );
    let chunk = part_bytes - OVERHEAD;
    if bytes.is_empty() {
        return vec![key.seal(&layout.asset_part(id, 0), &[])];
    }
    bytes.chunks(chunk).enumerate().map(|(i, c)| key.seal(&layout.asset_part(id, i), c)).collect()
}

/// Reassembles an asset from its part blobs, in order, and checks it is `size` bytes with
/// sha256 `sha256`. `id` names the asset in the error.
pub fn join<I>(key: &SyncKey, layout: &Layout<'_>, parts: I, id: &str, size: u64, sha256: &str) -> Result<Vec<u8>>
where
    I: IntoIterator,
    I::Item: AsRef<[u8]>,
{
    let mut out = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    for (i, part) in parts.into_iter().enumerate() {
        out.extend_from_slice(&key.open(&layout.asset_part(id, i), part.as_ref())?);
    }
    if out.len() as u64 != size {
        return Err(Error::Corrupt {
            id: id.to_owned(),
            detail: format!("{} bytes, expected {size}", out.len()),
        });
    }
    let got = sha256_hex(&out);
    if !got.eq_ignore_ascii_case(sha256) {
        return Err(Error::Corrupt {
            id: id.to_owned(),
            detail: format!("sha256 {got}, expected {sha256}"),
        });
    }
    Ok(out)
}
