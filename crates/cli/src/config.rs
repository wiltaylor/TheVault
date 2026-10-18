//! `cli.env`: where the agent install script leaves the server URL and token, so an agent's shell
//! runs `vault` with nothing set (ticket 131).
//!
//! The file is `$XDG_CONFIG_HOME/thevault/cli.env`, or `~/.config/thevault/cli.env`, of
//! `KEY=VALUE` lines. Only `VAULT_URL` and `VAULT_TOKEN` are read, and each only when neither its
//! flag nor its environment variable gave one. Blank lines and `#` comments are skipped, and a
//! value may be quoted.

use std::path::{Path, PathBuf};

/// Where `cli.env` is, from `XDG_CONFIG_HOME` and `HOME`.
pub fn path(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = xdg_config_home
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            home.filter(|p| !p.as_os_str().is_empty())
                .map(|h| h.join(".config"))
        })?;
    Some(base.join("thevault").join("cli.env"))
}

/// The `KEY=VALUE` pairs of `text`.
pub fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let l = l.strip_prefix("export ").unwrap_or(l);
            let (key, value) = l.split_once('=')?;
            let value = value.trim();
            let value = [('"', '"'), ('\'', '\'')]
                .iter()
                .find_map(|(a, b)| value.strip_prefix(*a)?.strip_suffix(*b))
                .unwrap_or(value);
            Some((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

/// The value `cli.env` at `path` gives `key`, if the file exists and names it with a value.
pub fn lookup(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    parse(&text)
        .into_iter()
        .rev()
        .find(|(k, v)| k == key && !v.is_empty())
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_lives_under_the_config_folder() {
        assert_eq!(
            path(Some("/x".into()), Some("/home/u".into())),
            Some(PathBuf::from("/x/thevault/cli.env"))
        );
        assert_eq!(
            path(Some("".into()), Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/thevault/cli.env"))
        );
        assert_eq!(path(None, None), None);
    }

    #[test]
    fn lines_are_key_value_with_optional_quotes() {
        let got = parse(
            "# written by install.sh\n\nVAULT_URL=http://a:1\nexport VAULT_TOKEN=\"tv_x\"\nB='q'\nnoise\n",
        );
        assert_eq!(
            got,
            [
                ("VAULT_URL".to_owned(), "http://a:1".to_owned()),
                ("VAULT_TOKEN".to_owned(), "tv_x".to_owned()),
                ("B".to_owned(), "q".to_owned()),
            ]
        );
    }
}
