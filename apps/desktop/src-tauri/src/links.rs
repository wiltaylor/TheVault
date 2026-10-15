//! `thevault://` links (D12). The `.desktop` file registers the app as the handler of the
//! `x-scheme-handler/thevault` MIME type, so the desktop starts `thevault thevault://note/<id>`,
//! and a running app gets the link through the single-instance plugin.
//!
//! The one link kind is `thevault://note/<id>`, which opens the note with that ULID.

/// The URI scheme the app handles.
pub const SCHEME: &str = "thevault";

/// The app's id: its desktop file is `<APP_ID>.desktop`, and the portal and the notification
/// daemon know it by this name.
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

/// The launcher the app writes for itself when it runs from an AppImage, which installs no
/// desktop file: it names the AppImage as the handler of `thevault://` links. The deb and rpm
/// packages install theirs from `linux/thevault.desktop`.
pub fn register_appimage() {
    let Some(appimage) = std::env::var_os("APPIMAGE") else {
        return;
    };
    let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".local/share"))
        })
    else {
        return;
    };
    let dir = data.join("applications");
    let entry = launcher_entry(&std::path::PathBuf::from(appimage));
    if let Err(e) = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(format!("{APP_ID}.desktop")), entry))
    {
        eprintln!("thevault: registering thevault:// links: {e}");
        return;
    }
    for (cmd, args) in [
        (
            "update-desktop-database",
            vec![dir.to_string_lossy().into_owned()],
        ),
        (
            "xdg-mime",
            vec![
                "default".to_owned(),
                format!("{APP_ID}.desktop"),
                format!("x-scheme-handler/{SCHEME}"),
            ],
        ),
    ] {
        let _ = std::process::Command::new(cmd).args(args).status();
    }
}

/// The launcher entry that starts `exe` for `thevault://` links.
fn launcher_entry(exe: &std::path::Path) -> String {
    let exe = crate::settings::launcher_exec(exe);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TheVault\n\
         Comment=Notes, offline, synced with your TheVault server\n\
         Exec={exe} %u\n\
         Icon=thevault\n\
         Terminal=false\n\
         Categories=Office;\n\
         StartupWMClass=thevault\n\
         MimeType=x-scheme-handler/{SCHEME};\n"
    )
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
    fn the_appimage_launcher_handles_thevault_links() {
        let entry = launcher_entry(std::path::Path::new("/home/u/Apps/TheVault.AppImage"));
        assert!(
            entry.contains("Exec=/home/u/Apps/TheVault.AppImage %u\n"),
            "{entry}"
        );
        assert!(
            entry.contains("MimeType=x-scheme-handler/thevault;\n"),
            "{entry}"
        );
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
