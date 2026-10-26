//! Plain types that the script engine and the server crates both name, with no dependencies, so
//! `script-core` can name them without reaching `state` or `index` (D6).

/// The version of the sync protocol: `GET /api/sync/info`, `GET /api/sync/changes` and the device
/// bases. A device refuses to sync with a server whose protocol is newer than it knows (D27).
pub const SYNC_PROTOCOL: u32 = 1;

/// The version of the vault file format in `docs/vault-format.md`. A device refuses to sync with a
/// server whose format is newer than it knows, since it could write notes the server reads wrong
/// (D27). Raise it with any change an older reader would misread.
///
/// 2: agent files, `agents/<name>.wcl` (agents G1), a synced folder an older device refuses.
/// 3: skills, `skills/<name>/` (skills S1): a synced folder an older device refuses, whose binary
/// files the feed carries as `base64`, which an older device would read as deletes.
/// 4: typed lists and dicts (types T6): `kind = "list"` of any kind in place of `multiple`, and
/// `key` blocks in a dict, which an older reader refuses, failing the whole schema; and the
/// `card` content block, which an older reader refuses in a note; and lists of dicts that hold
/// each value with its label, a person's `phones`, `emails` and `addresses` among them, in place
/// of the `labels` attribute naming a label list; and the `time` field kind, a time of day with
/// no date, which an older reader refuses as an unknown kind.
/// 5: AI accounts (accounts A10): agent files name `account = "<account name>"` in place of
/// `provider`, which an older reader refuses as an unknown agent field (its `provider` is a
/// closed list of three); and the planner's built-in `weeklynote`, `monthlynote`,
/// `quarterlynote` and `yearlynote` types, whose templates hold the template-only
/// `period_calendar` block, an unknown block that makes an older reader refuse the type.
/// 6: script tools (script-tools T2, T4, T7, T8): the synced `tools/` folder of tool files,
/// `tools/<name>.wcl`, an unknown folder an older device refuses; a skill's own `tools/` and
/// `scripts/`; and the agent permission keys `permissions { scripts { run_code } }` and
/// `confirm { tools }`, which an older reader refuses as an unknown permissions block and switch;
/// and sub agents (subagents U5): the `permissions { subagents { allow, max, agents } }` block,
/// an unknown permissions block to an older reader, and a thread's sub agent sessions, origin
/// `subagent:<thread>` with `parent` and `subagents` in its transcript.
/// 7: the agent permission key `permissions { scripts { run_saved } }` (Run saved scripts and
/// tasks), which an older reader refuses as an unknown switch of `scripts`, failing the agent
/// file.
pub const FORMAT_VERSION: u32 = 7;

/// Who made a change (D13, D30).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Author {
    /// A browser session.
    Session(String),
    /// A named API token.
    Token(String),
    /// A named wscript handler.
    Handler(String),
    /// A named scheduled task (D10).
    Task(String),
    /// A named agent (agents G11): every change an agent's tools make, whether it applies them
    /// itself or the user applies its proposal. Its writes never fire note event triggers.
    Agent(String),
}

impl Author {
    /// The author's kind (`session`, `token`, `handler`, `task` or `agent`) and name.
    pub fn parts(&self) -> (&'static str, &str) {
        match self {
            Author::Session(n) => ("session", n),
            Author::Token(n) => ("token", n),
            Author::Handler(n) => ("handler", n),
            Author::Task(n) => ("task", n),
            Author::Agent(n) => ("agent", n),
        }
    }

    /// The author of `kind` named `name`, or `None` for an unknown kind.
    pub fn from_parts(kind: &str, name: String) -> Option<Author> {
        match kind {
            "session" => Some(Author::Session(name)),
            "token" => Some(Author::Token(name)),
            "handler" => Some(Author::Handler(name)),
            "task" => Some(Author::Task(name)),
            "agent" => Some(Author::Agent(name)),
            _ => None,
        }
    }
}

/// Which notes a listing or search reads, by whether they are archived. A note is archived when
/// it holds `archived = true`; listings leave archived notes out unless they ask for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Archived {
    /// Only notes that are not archived: the default.
    #[default]
    Exclude,
    /// Every note, archived or not.
    Include,
    /// Only archived notes.
    Only,
}

impl Archived {
    /// `exclude`, `include` or `only`.
    pub fn parse(s: &str) -> Option<Archived> {
        Some(match s {
            "exclude" => Archived::Exclude,
            "include" => Archived::Include,
            "only" => Archived::Only,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Archived::Exclude => "exclude",
            Archived::Include => "include",
            Archived::Only => "only",
        }
    }

    /// An SQL condition on `column`, the `archived` column of `notes`.
    pub fn sql(self, column: &str) -> String {
        match self {
            Archived::Exclude => format!("{column} = 0"),
            Archived::Include => "1".to_owned(),
            Archived::Only => format!("{column} = 1"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_author_round_trips_through_its_parts() {
        for a in [
            Author::Session("owner".into()),
            Author::Token("ci".into()),
            Author::Handler("memo.after_create".into()),
            Author::Task("digest".into()),
            Author::Agent("assistant".into()),
        ] {
            let (kind, name) = a.parts();
            assert_eq!(Author::from_parts(kind, name.to_owned()), Some(a.clone()));
        }
        assert_eq!(Author::Agent("triage".into()).parts(), ("agent", "triage"));
        assert_eq!(Author::from_parts("robot", "x".into()), None);
    }
}
