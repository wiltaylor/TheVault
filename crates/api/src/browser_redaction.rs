//! Server-only credentials. Never derive Debug or expose the pattern list.
use hmac::{Hmac, Mac};

pub(crate) struct Redactor(Vec<(String, bool)>);

fn seed_parameters(input: &str, mut digits: u32, mut period: u64) -> Option<(String, u32, u64)> {
    let mut seed = input.to_owned();
    if input.trim().to_ascii_lowercase().starts_with("otpauth://") {
        let uri = reqwest::Url::parse(input.trim()).ok()?;
        if uri.scheme() != "otpauth" || uri.host_str() != Some("totp") { return None; }
        let mut secret = None;
        for (key, value) in uri.query_pairs() {
            match key.as_ref() {
                "secret" => { if secret.is_some() { return None; } secret = Some(value.into_owned()); }
                "digits" => digits = value.parse().ok()?,
                "period" => period = value.parse().ok()?,
                "algorithm" if !value.eq_ignore_ascii_case("SHA1") => return None,
                _ => {}
            }
        }
        seed = secret?;
    }
    let seed = seed.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_ascii_uppercase();
    if !(6..=8).contains(&digits) || period == 0 || period > i64::MAX as u64 / 2 { return None; }
    Some((seed, digits, period))
}

/// RFC 6238 SHA-1 TOTP. Raw seeds default to the supplied digits and period.
pub(crate) fn totp(seed: &str, seconds: u64, digits: u32, period: u64) -> Option<String> {
    let (seed, digits, period) = seed_parameters(seed, digits, period)?;
    let mut key = Vec::new();
    let (mut bits, mut count) = (0u32, 0u32);
    for c in seed.bytes().take_while(|c| *c != b'=') {
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
    if key.is_empty() {
        return None;
    }
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&key).ok()?;
    mac.update(&(seconds / period).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 15);
    let number = u32::from_be_bytes(hash[offset..offset + 4].try_into().ok()?) & 0x7fff_ffff;
    Some(format!(
        "{:0width$}",
        number % 10u32.pow(digits),
        width = digits as usize
    ))
}

impl Redactor {
    pub(crate) fn load(state: &state::State) -> Result<Self, ()> {
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
                let codes = seed_parameters(seed, 6, 30).and_then(|(seed, digits, period)| {
                    (-2i64..=2).map(|step| totp(&seed, now.saturating_add_signed(step * period as i64), digits, period)).collect::<Option<Vec<_>>>()
                });
                if let Some(codes) = codes {
                    patterns.extend(codes.into_iter().map(|code| (code, true)));
                } else {
                    eprintln!("browser: computed-code redaction skipped for Login {:?}: invalid TOTP configuration", info.name);
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
    #[test]
    fn normalized_seed_and_uri_parameters() {
        let seed = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        assert_eq!(totp("gezd gnbv-gy3tqojq gezdgnbvgy3tqojq", 59, 8, 30), totp(seed, 59, 8, 30));
        assert_eq!(totp(&format!("otpauth://totp/test?secret={seed}&digits=8&period=60"), 119, 6, 30).as_deref(), Some("94287082"));
    }
    #[test]
    fn invalid_seed_keeps_raw_redaction_available() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::State::open(&dir.path().join("state.db"), Some(state::SecretKey::generate())).unwrap();
        state.set_login_secret("broken", &state::LoginSecret { username:"fake-user".into(), password:"fake-password".into(), totp_seed:Some("not a valid seed!".into()), origins:vec![] }).unwrap();
        let r = Redactor::load(&state).ok().expect("invalid seed must not block browser output");
        assert_eq!(r.text("fake-user fake-password not a valid seed!"), "[REDACTED] [REDACTED] [REDACTED]");
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
