//! `thevault://` links (D12). The `.desktop` file registers the app as the handler of the
//! `x-scheme-handler/thevault` MIME type, so the desktop starts `thevault thevault://note/<id>`,
//! and a running app gets the link through the single-instance plugin. The packages install that
//! file from `linux/thevault.desktop`; an AppImage writes its own ([`crate::app_menu`]).
//!
//! The one link kind is `thevault://note/<id>`, which opens the note with that ULID.

/// The URI scheme the app handles.
pub const SCHEME: &str = "thevault";

/// The app's id, the identifier in `tauri.conf.json`: the Wayland `app_id` of its window
/// ([`crate::app_menu::name_program`]), the name of the AppImage's desktop file, `<APP_ID>.desktop`, and the
/// packages' `StartupWMClass`. The portal and the notification daemon know it by this name.
pub const APP_ID: &str = "dev.wiltaylor.thevault";

/// What a `thevault://` link asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Open the note with this id.
    Note(String),
}

impl Link {
    /// The web client's path that shows what the link points at.
    pub fn path(&self) -> String {
        match self {
            Link::Note(id) => format!("/notes/{id}"),
        }
    }
}

/// The link `arg` names, or `None` when it is not a `thevault://` link the app knows.
///
/// The scheme and the `note` kind match in any case, as URIs do, and the id is upper-cased, as
/// ULIDs are written. A trailing slash, a query and a fragment are ignored.
pub fn parse(arg: &str) -> Option<Link> {
    let arg = arg.trim();
    let (scheme, rest) = arg.split_once("://")?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let (kind, id) = rest.split_once('/')?;
    if !kind.eq_ignore_ascii_case("note") {
        return None;
    }
    is_ulid(id).then(|| Link::Note(id.to_ascii_uppercase()))
}

/// The first `thevault://` link among the command-line arguments.
pub fn from_args<S: AsRef<str>>(args: &[S]) -> Option<Link> {
    args.iter().find_map(|a| parse(a.as_ref()))
}

/// Whether `id` is a ULID: 26 characters of Crockford base 32, the first at most `7`.
fn is_ulid(id: &str) -> bool {
    const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    id.len() == 26
        && id.as_bytes()[0] <= b'7'
        && id
            .chars()
            .all(|c| ALPHABET.contains(c.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01J9ZQ4V6W3K8M2N5P7R9T1XYZ";

    #[test]
    fn a_note_link_opens_the_note() {
        let link = parse(&format!("thevault://note/{ID}")).unwrap();
        assert_eq!(link, Link::Note(ID.to_owned()));
        assert_eq!(link.path(), format!("/notes/{ID}"));
    }

    #[test]
    fn case_a_trailing_slash_a_query_and_a_fragment_do_not_matter() {
        let want = Some(Link::Note(ID.to_owned()));
        assert_eq!(parse(&format!("TheVault://Note/{ID}")), want);
        assert_eq!(
            parse(&format!("thevault://note/{}", ID.to_lowercase())),
            want
        );
        assert_eq!(parse(&format!("thevault://note/{ID}/")), want);
        assert_eq!(parse(&format!("thevault://note/{ID}?from=mail#top")), want);
        assert_eq!(parse(&format!("  thevault://note/{ID}\n")), want);
    }

    #[test]
    fn other_links_are_not_ours() {
        for link in [
            "".to_owned(),
            "--capture".to_owned(),
            format!("https://note/{ID}"),
            format!("thevault:note/{ID}"),
            format!("thevault:///note/{ID}"),
            format!("thevault://notes/{ID}"),
            format!("thevault://note/{ID}/extra"),
            "thevault://note/".to_owned(),
            "thevault://note".to_owned(),
            // Not a ULID: too short, too long, a letter Crockford leaves out, an overflowing first
            // character, and a path escape.
            "thevault://note/01J9ZQ4V6W".to_owned(),
            format!("thevault://note/{ID}0"),
            "thevault://note/01J9ZQ4V6W3K8M2N5P7R9T1XYU".to_owned(),
            "thevault://note/81J9ZQ4V6W3K8M2N5P7R9T1XYZ".to_owned(),
            "thevault://note/../../_desktop/setup".to_owned(),
        ] {
            assert_eq!(parse(&link), None, "{link:?}");
        }
    }

    #[test]
    fn the_first_link_among_the_arguments_wins() {
        let args = [
            "thevault".to_owned(),
            "--minimised".to_owned(),
            format!("thevault://note/{ID}"),
            "thevault://note/01J9ZQ4V6W3K8M2N5P7R9T1XY0".to_owned(),
        ];
        assert_eq!(from_args(&args), Some(Link::Note(ID.to_owned())));
        assert_eq!(from_args(&["thevault", "--capture"]), None);
    }
}
