//! Server-only credentials. Never derive Debug or expose the pattern list.
use hmac::{Hmac, Mac};

#[cfg(test)]
mod h7_tests {
    use super::totp;

    #[test]
    fn uri_algorithm_codes_are_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::State::open(
            &dir.path().join("state.db"),
            Some(state::SecretKey::generate()),
        )
        .unwrap();
        for algorithm in ["SHA256", "SHA512"] {
            let seed = format!(
                "otpauth://totp/test?secret=JBSWY3DPEHPK3PXP&algorithm={algorithm}&digits=8&period=60"
            );
            state
                .set_login_secret(
                    "test",
                    &state::LoginSecret {
                        username: "fake-user".into(),
                        password: "fake-password".into(),
                        totp_seed: Some(seed.clone()),
                        origins: vec![],
                    },
                )
                .unwrap();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let code = totp(&seed, now, 6, 30).expect("supported URI");
            assert_eq!(
                super::Redactor::load(&state)
                    .unwrap_or_else(|_| panic!("load failed"))
                    .text(&code),
                "[REDACTED]"
            );
        }
    }

    #[test]
    fn rfc_hashes_and_strict_parameters() {
        for (algorithm, seed, expected) in [
            (
                "SHA256",
                "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA",
                "46119246",
            ),
            (
                "SHA512",
                "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNA",
                "90693936",
            ),
        ] {
            let uri = format!(
                "otpauth://totp/test?secret={seed}&algorithm={algorithm}&digits=8&period=30"
            );
            assert_eq!(totp(&uri, 59, 6, 30).as_deref(), Some(expected));
            let remaining = if algorithm == "SHA256" {
                ["68084774", "67062674", "91819424", "90698825", "77737706"]
            } else {
                ["25091201", "99943326", "93441116", "38618901", "47863826"]
            };
            for (time, expected) in [1111111109, 1111111111, 1234567890, 2000000000, 20000000000]
                .into_iter()
                .zip(remaining)
            {
                assert_eq!(totp(&uri, time, 6, 30).as_deref(), Some(expected));
            }
            let uri60 = uri.replace("period=30", "period=60");
            assert_eq!(totp(&uri60, 119, 6, 30), totp(&uri, 59, 6, 30));
            assert_ne!(totp(&uri60, 120, 6, 30), totp(&uri60, 119, 6, 30));
        }
        for seed in [
            "MZ",
            "MY=",
            "MY======junk",
            "ABC",
            "otpauth://totp/test?secret=MY&digits=6&digits=8",
            "otpauth://totp/test?secret=MY&period=30&period=60",
        ] {
            assert!(
                totp(seed, 59, 6, 30).is_none(),
                "invalid configuration accepted"
            );
        }
    }
}

pub(crate) struct Redactor(Vec<(String, bool)>);

// Current-thread async tests observe the actual credential-read boundary across
// network awaits. No credential data is recorded, and Drop isolates later tests.
#[cfg(test)]
thread_local! {
    static CONTEXT_READS: std::cell::RefCell<Option<Vec<Option<script::RunContext>>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct ContextReads;

#[cfg(test)]
impl ContextReads {
    pub(crate) fn start() -> Self {
        CONTEXT_READS.with(|reads| *reads.borrow_mut() = Some(Vec::new()));
        Self
    }

    pub(crate) fn take(&self) -> Vec<Option<script::RunContext>> {
        CONTEXT_READS.with(|reads| std::mem::take(reads.borrow_mut().as_mut().unwrap()))
    }
}

#[cfg(test)]
impl Drop for ContextReads {
    fn drop(&mut self) {
        CONTEXT_READS.with(|reads| *reads.borrow_mut() = None);
    }
}

fn seed_parameters(
    input: &str,
    mut digits: u32,
    mut period: u64,
) -> Option<(String, u32, u64, String)> {
    let mut seed = input.to_owned();
    let mut algorithm = "SHA1".to_owned();
    if input.trim().to_ascii_lowercase().starts_with("otpauth://") {
        let uri = reqwest::Url::parse(input.trim()).ok()?;
        if uri.scheme() != "otpauth"
            || uri.host_str() != Some("totp")
            || !uri.username().is_empty()
            || uri.password().is_some()
            || uri.port().is_some()
            || uri.fragment().is_some()
        {
            return None;
        }
        let mut secret = None;
        let mut seen = std::collections::HashSet::new();
        for (key, value) in uri.query_pairs() {
            if matches!(key.as_ref(), "secret" | "digits" | "period" | "algorithm")
                && !seen.insert(key.to_string())
            {
                return None;
            }
            match key.as_ref() {
                "secret" => {
                    if secret.is_some() {
                        return None;
                    }
                    secret = Some(value.into_owned());
                }
                "digits" => digits = value.parse().ok()?,
                "period" => period = value.parse().ok()?,
                "algorithm" => algorithm = value.to_ascii_uppercase(),
                _ => {}
            }
        }
        seed = secret?;
    }
    let seed = seed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    if !(6..=8).contains(&digits) || period == 0 || period > i64::MAX as u64 / 2 {
        return None;
    }
    if !matches!(algorithm.as_str(), "SHA1" | "SHA256" | "SHA512") {
        return None;
    }
    Some((seed, digits, period, algorithm))
}

/// RFC 6238 TOTP. Raw seeds default to SHA1 and the supplied digits and period.
pub(crate) fn totp(seed: &str, seconds: u64, digits: u32, period: u64) -> Option<String> {
    let (seed, digits, period, algorithm) = seed_parameters(seed, digits, period)?;
    let unpadded = seed.trim_end_matches('=');
    let remainder = unpadded.len() % 8;
    let padding = seed.len() - unpadded.len();
    if !matches!(remainder, 0 | 2 | 4 | 5 | 7) || (padding != 0 && padding != (8 - remainder) % 8) {
        return None;
    }
    let mut key = Vec::new();
    let (mut bits, mut count) = (0u32, 0u32);
    for c in unpadded.bytes() {
        let c = c.to_ascii_uppercase();
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | u32::from(value);
        count += 5;
        if count >= 8 {
            count -= 8;
            key.push((bits >> count) as u8);
        }
    }
    if key.is_empty() || (bits & ((1 << count) - 1)) != 0 {
        return None;
    }
    macro_rules! digest {
        ($hash:ty) => {{
            let mut mac = Hmac::<$hash>::new_from_slice(&key).ok()?;
            mac.update(&(seconds / period).to_be_bytes());
            mac.finalize().into_bytes().to_vec()
        }};
    }
    let hash = match algorithm.as_str() {
        "SHA256" => digest!(sha2::Sha256),
        "SHA512" => digest!(sha2::Sha512),
        _ => digest!(sha1::Sha1),
    };
    let offset = usize::from(hash.last()? & 15);
    let number = u32::from_be_bytes(hash[offset..offset + 4].try_into().ok()?) & 0x7fff_ffff;
    Some(format!(
        "{:0width$}",
        number % 10u32.pow(digits),
        width = digits as usize
    ))
}

impl Redactor {
    pub(crate) fn load(state: &state::State) -> Result<Self, ()> {
        #[cfg(test)]
        CONTEXT_READS.with(|reads| {
            if let Some(reads) = reads.borrow_mut().as_mut() {
                reads.push(script::RunContext::current());
            }
        });
        let mut patterns = Vec::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ())?
            .as_secs();
        for info in state.list_login_secrets().map_err(|_| ())? {
            let login = state
                .get_login_secret(&info.name)
                .map_err(|_| ())?
                .ok_or(())?;
            if let Some(seed) = &login.totp_seed {
                let codes =
                    seed_parameters(seed, 6, 30).and_then(|(normalized, digits, period, _)| {
                        let raw = seed;
                        let seed = normalized;
                        let grouped = seed
                            .as_bytes()
                            .chunks(4)
                            .map(String::from_utf8_lossy)
                            .collect::<Vec<_>>()
                            .join(" ");
                        for value in [
                            seed.clone(),
                            seed.to_ascii_lowercase(),
                            grouped.clone(),
                            grouped.to_ascii_lowercase(),
                        ] {
                            if !value.is_empty() {
                                let short = value.chars().count() <= 3;
                                patterns.push((value, short));
                            }
                        }
                        (-2i64..=2)
                            .map(|step| {
                                totp(
                                    raw,
                                    now.saturating_add_signed(step * period as i64),
                                    digits,
                                    period,
                                )
                            })
                            .collect::<Option<Vec<_>>>()
                    });
                if let Some(codes) = codes {
                    patterns.extend(codes.into_iter().map(|code| (code, true)));
                } else {
                    eprintln!(
                        "browser: computed-code redaction skipped for Login {:?}: invalid TOTP configuration",
                        info.name
                    );
                }
            }
            for value in [Some(login.username), Some(login.password), login.totp_seed]
                .into_iter()
                .flatten()
            {
                if !value.is_empty() {
                    let short = value.chars().count() <= 3;
                    patterns.push((value, short));
                }
            }
        }
        patterns.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
        Ok(Self(patterns))
    }

    pub(crate) fn text(&self, input: &str) -> String {
        fn word(c: char) -> bool {
            c.is_alphanumeric() || c == '_'
        }
        let mut output = String::new();
        let mut offset = 0;
        while offset < input.len() {
            let rest = &input[offset..];
            let found = self.0.iter().find(|(pattern, token)| {
                rest.starts_with(pattern)
                    && (!token
                        || (!input[..offset].chars().next_back().is_some_and(word)
                            && !rest[pattern.len()..].chars().next().is_some_and(word)))
            });
            if let Some((pattern, _)) = found {
                output.push_str("[REDACTED]");
                offset += pattern.len();
            } else {
                let c = rest.chars().next().expect("nonempty");
                output.push(c);
                offset += c.len_utf8();
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assert_seed_forms_redacted(stored: &str) {
        let dir = tempfile::tempdir().unwrap();
        let state = state::State::open(
            &dir.path().join("state.db"),
            Some(state::SecretKey::generate()),
        )
        .unwrap();
        state
            .set_login_secret(
                "fabricated",
                &state::LoginSecret {
                    username: "fake-user".into(),
                    password: "fake-password".into(),
                    totp_seed: Some(stored.into()),
                    origins: vec![],
                },
            )
            .unwrap();
        let r = Redactor::load(&state).unwrap();
        for displayed in [
            stored,
            "JBSWY3DPEHPK3PXP",
            "jbswy3dpehpk3pxp",
            "JBSW Y3DP EHPK 3PXP",
            "jbsw y3dp ehpk 3pxp",
        ] {
            assert_eq!(r.text(displayed), "[REDACTED]");
        }
        assert_eq!(r.text("ordinary setup page"), "ordinary setup page");
    }
    #[test]
    fn spaced_seed_redacts_normalized_forms() {
        assert_seed_forms_redacted("JBSW Y3DP EHPK 3PXP");
    }
    #[test]
    fn hyphenated_seed_redacts_normalized_forms() {
        assert_seed_forms_redacted("JBSW-Y3DP-EHPK-3PXP");
    }
    #[test]
    fn lowercase_seed_redacts_normalized_forms() {
        assert_seed_forms_redacted("jbswy3dpehpk3pxp");
    }
    #[test]
    fn uri_seed_redacts_normalized_forms() {
        assert_seed_forms_redacted(
            "otpauth://totp/test?secret=JBSWY3DPEHPK3PXP&digits=8&period=60",
        );
    }
    #[test]
    fn normalized_seed_and_uri_parameters() {
        let seed = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        assert_eq!(
            totp("gezd gnbv-gy3tqojq gezdgnbvgy3tqojq", 59, 8, 30),
            totp(seed, 59, 8, 30)
        );
        assert_eq!(
            totp(
                &format!("otpauth://totp/test?secret={seed}&digits=8&period=60"),
                119,
                6,
                30
            )
            .as_deref(),
            Some("94287082")
        );
    }
    #[test]
    fn invalid_seed_keeps_raw_redaction_available() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::State::open(
            &dir.path().join("state.db"),
            Some(state::SecretKey::generate()),
        )
        .unwrap();
        state
            .set_login_secret(
                "broken",
                &state::LoginSecret {
                    username: "fake-user".into(),
                    password: "fake-password".into(),
                    totp_seed: Some("not a valid seed!".into()),
                    origins: vec![],
                },
            )
            .unwrap();
        let r = Redactor::load(&state).expect("invalid seed must not block browser output");
        assert_eq!(
            r.text("fake-user fake-password not a valid seed!"),
            "[REDACTED] [REDACTED] [REDACTED]"
        );
    }
    #[test]
    fn rfc6238_vectors() {
        for (time, expected) in [
            (59, "94287082"),
            (1111111109, "07081804"),
            (1111111111, "14050471"),
            (1234567890, "89005924"),
            (2000000000, "69279037"),
            (20000000000, "65353130"),
        ] {
            assert_eq!(
                totp("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", time, 8, 30).as_deref(),
                Some(expected)
            );
        }
    }
    #[test]
    fn short_tokens_and_overlap() {
        let r = Redactor(vec![
            ("abc-long".into(), false),
            ("abc".into(), true),
            ("x".into(), true),
        ]);
        assert_eq!(
            r.text("abc-long abc alphabet x extra abcdef"),
            "[REDACTED] [REDACTED] alphabet [REDACTED] extra abcdef"
        );
    }
}
