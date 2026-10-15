//! Plain types that the script engine and the server crates both name, with no dependencies, so
//! `script-core` can name them without reaching `state` or `index` (D6).

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
}

impl Author {
    /// The author's kind (`session`, `token`, `handler` or `task`) and name.
    pub fn parts(&self) -> (&'static str, &str) {
        match self {
            Author::Session(n) => ("session", n),
            Author::Token(n) => ("token", n),
            Author::Handler(n) => ("handler", n),
            Author::Task(n) => ("task", n),
        }
    }

    /// The author of `kind` named `name`, or `None` for an unknown kind.
    pub fn from_parts(kind: &str, name: String) -> Option<Author> {
        match kind {
            "session" => Some(Author::Session(name)),
            "token" => Some(Author::Token(name)),
            "handler" => Some(Author::Handler(name)),
            "task" => Some(Author::Task(name)),
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
