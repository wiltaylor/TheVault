//! The S6 vault generator: a vault of any size whose every file depends only on a seed.
//!
//! The generator writes canonical files straight into the vault folder through `store`, not
//! through the batch API. At 1M notes the API would spend its time on per-commit work the
//! benchmark is not measuring, and the reindex the benchmark times reads the folder either way.
//! Every note is put through [`store::canonical`], so the files are byte for byte what the writer
//! would have written, and the reindex validates every one of them against its type.
//!
//! The mix is four types: people, projects, tasks and articles. Tasks and projects are linked by a
//! two-way reference, tasks and projects name an owner or assignee, articles form peer groups of
//! three, and every type uses the hierarchical `status` or `topic` taxonomy. Content paragraphs
//! draw from a skewed vocabulary of made-up words, so full-text search has common and rare terms,
//! and some paragraphs mention another note.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};

use crate::rng::Rng;

/// The type and taxonomy files every generated vault holds.
const DEFINITIONS: &[(&str, &str)] = &[
    (
        "taxonomies/status.wcl",
        include_str!("../defs/taxonomies/status.wcl"),
    ),
    (
        "taxonomies/topic.wcl",
        include_str!("../defs/taxonomies/topic.wcl"),
    ),
    ("types/person.wcl", include_str!("../defs/types/person.wcl")),
    (
        "types/project.wcl",
        include_str!("../defs/types/project.wcl"),
    ),
    ("types/task.wcl", include_str!("../defs/types/task.wcl")),
    (
        "types/article.wcl",
        include_str!("../defs/types/article.wcl"),
    ),
];

/// Every term of `taxonomies/status.wcl`, with how often a task holds it, out of 100.
const STATUSES: &[(&str, u64)] = &[
    ("Inbox", 15),
    ("Active", 5),
    ("Active/In progress", 20),
    ("Active/Blocked", 5),
    ("Active/Review", 5),
    ("Done", 40),
    ("Archived", 10),
];

/// The roots and children of `taxonomies/topic.wcl`.
pub const TOPICS: &[(&str, [&str; 6])] = &[
    (
        "Science",
        [
            "Physics",
            "Chemistry",
            "Biology",
            "Astronomy",
            "Geology",
            "Maths",
        ],
    ),
    (
        "Tech",
        [
            "Rust",
            "Web",
            "Databases",
            "Networking",
            "Security",
            "Hardware",
        ],
    ),
    (
        "Arts",
        ["Music", "Painting", "Film", "Theatre", "Poetry", "Design"],
    ),
    (
        "History",
        [
            "Ancient", "Medieval", "Modern", "Military", "Economic", "Social",
        ],
    ),
    (
        "Business",
        [
            "Finance",
            "Marketing",
            "Strategy",
            "Operations",
            "Legal",
            "Sales",
        ],
    ),
    (
        "Health",
        [
            "Fitness",
            "Nutrition",
            "Sleep",
            "Medicine",
            "Mental",
            "Recovery",
        ],
    ),
    (
        "Travel",
        ["Europe", "Asia", "Africa", "Americas", "Oceania", "Polar"],
    ),
    (
        "Food",
        [
            "Baking",
            "Grilling",
            "Fermentation",
            "Vegetables",
            "Seafood",
            "Desserts",
        ],
    ),
];

const PRIORITIES: &[&str] = &["low", "medium", "high"];

/// Words in the content vocabulary.
const VOCABULARY: usize = 20_000;

/// 2019-01-01T00:00:00Z: the earliest `created` time.
const EPOCH_MS: u64 = 1_546_300_800_000;
/// Seven years: notes are created across this span after [`EPOCH_MS`].
const SPAN_MS: u64 = 7 * 365 * 86_400_000;

/// Salts that keep the random streams of the plan, the links and the content apart.
const ID_SALT: u64 = 0x1D5;
const LINK_SALT: u64 = 0x11_4C;
const TEXT_SALT: u64 = 0x7E_C7;

/// The note types, in the order their notes are numbered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Person,
    Project,
    Task,
    Article,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Person, Kind::Project, Kind::Task, Kind::Article];

    pub fn type_name(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Project => "project",
            Kind::Task => "task",
            Kind::Article => "article",
        }
    }
}

/// What to generate.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub notes: usize,
    pub seed: u64,
}

/// Every note's id and every link between notes, decided before any file is written so that the
/// files can be written in parallel and still agree with one another.
pub struct Plan {
    pub spec: Spec,
    /// The number of notes of each kind, in [`Kind::ALL`] order.
    counts: [usize; 4],
    ids: Vec<String>,
    created: Vec<u64>,
    /// The project of each task, by task number.
    task_project: Vec<Option<u32>>,
    /// The tasks of each project, by project number, in task order.
    project_tasks: Vec<Vec<u32>>,
    words: Vec<String>,
}

impl Plan {
    pub fn new(spec: Spec) -> Plan {
        let n = spec.notes;
        let people = n * 4 / 100;
        let projects = n * 6 / 100;
        let articles = n * 30 / 100;
        let tasks = n - people - projects - articles;
        let counts = [people, projects, tasks, articles];

        let mut ids = Vec::with_capacity(n);
        let mut created = Vec::with_capacity(n);
        for i in 0..n {
            let mut r = Rng::for_item(spec.seed ^ ID_SALT, i as u64);
            let ms = EPOCH_MS + r.below(SPAN_MS);
            let random =
                ((r.next_u64() as u128) << 64 | r.next_u64() as u128) & ((1u128 << 80) - 1);
            ids.push(ulid::Ulid::from_parts(ms, random).to_string());
            created.push(ms);
        }

        let mut task_project = Vec::with_capacity(tasks);
        let mut project_tasks = vec![Vec::new(); projects];
        for t in 0..tasks {
            let mut r = Rng::for_item(spec.seed ^ LINK_SALT, t as u64);
            let p = (projects > 0 && r.chance(0.9)).then(|| r.below(projects as u64) as u32);
            if let Some(p) = p {
                project_tasks[p as usize].push(t as u32);
            }
            task_project.push(p);
        }

        let mut r = Rng::new(spec.seed ^ TEXT_SALT);
        let words = (0..VOCABULARY).map(|_| word(&mut r)).collect();

        Plan {
            spec,
            counts,
            ids,
            created,
            task_project,
            project_tasks,
            words,
        }
    }

    pub fn count(&self, kind: Kind) -> usize {
        self.counts[kind as usize]
    }

    fn start(&self, kind: Kind) -> usize {
        self.counts[..kind as usize].iter().sum()
    }

    /// The kind of note `i` and its number among notes of that kind.
    fn kind_of(&self, i: usize) -> (Kind, usize) {
        let mut start = 0;
        for kind in Kind::ALL {
            let count = self.count(kind);
            if i < start + count {
                return (kind, i - start);
            }
            start += count;
        }
        unreachable!("note {i} is past the end of the plan")
    }

    /// The id of note number `n` of `kind`.
    pub fn id(&self, kind: Kind, n: usize) -> &str {
        &self.ids[self.start(kind) + n]
    }

    /// The ids of every note of `kind`.
    pub fn ids(&self, kind: Kind) -> &[String] {
        let start = self.start(kind);
        &self.ids[start..start + self.count(kind)]
    }

    /// Word `rank` of the vocabulary, where rank 0 is the most common.
    pub fn word(&self, rank: usize) -> &str {
        &self.words[rank]
    }

    /// The vault path of note `i`.
    fn path(&self, i: usize) -> String {
        let (kind, _) = self.kind_of(i);
        format!("notes/{}/{}.wcl", kind.type_name(), self.ids[i])
    }

    /// The WCL text of note `i`, before it is put in canonical form.
    pub fn note(&self, i: usize) -> String {
        let (kind, n) = self.kind_of(i);
        let mut r = Rng::for_item(self.spec.seed ^ TEXT_SALT, i as u64);
        let mut out = String::new();
        let created = self.created[i];
        let updated = created + r.below(365 * 86_400_000);
        item(&mut out, "title", &quote(&self.title(&mut r, kind)));
        item(&mut out, "created", &quote(&datetime(created)));
        item(&mut out, "updated", &quote(&datetime(updated)));
        let paragraphs = match kind {
            Kind::Person => {
                let first = capitalise(self.common(&mut r));
                let org = self.common(&mut r);
                item(
                    &mut out,
                    "email",
                    &quote(&format!("{}.{n}@{org}.example", first.to_lowercase())),
                );
                item(&mut out, "organisation", &quote(&capitalise(org)));
                if r.chance(0.5) {
                    item(
                        &mut out,
                        "homepage",
                        &quote(&format!("https://{org}.example/people/{n}")),
                    );
                }
                usize::from(r.chance(0.6))
            }
            Kind::Project => {
                item(&mut out, "status", &quote(self.status(&mut r)));
                if self.count(Kind::Person) > 0 && r.chance(0.9) {
                    let owner = r.below(self.count(Kind::Person) as u64) as usize;
                    item(&mut out, "owner", &quote(self.id(Kind::Person, owner)));
                }
                if r.chance(0.8) {
                    item(&mut out, "due", &quote(&self.date_after(&mut r, created)));
                }
                item(&mut out, "budget", &format!("{}.0", r.range(1, 500) * 500));
                item(&mut out, "topics", &list(&topics(&mut r)));
                let tasks: Vec<String> = self.project_tasks[n]
                    .iter()
                    .map(|&t| self.id(Kind::Task, t as usize).to_owned())
                    .collect();
                if !tasks.is_empty() {
                    item(&mut out, "tasks", &list(&tasks));
                }
                r.range(1, 3) as usize
            }
            Kind::Task => {
                let status = self.status(&mut r);
                item(&mut out, "status", &quote(status));
                item(&mut out, "priority", &quote(r.pick(PRIORITIES)));
                if r.chance(0.85) {
                    item(&mut out, "due", &quote(&self.date_after(&mut r, created)));
                }
                if r.chance(0.7) {
                    item(&mut out, "estimate", &r.range(1, 13).to_string());
                }
                item(
                    &mut out,
                    "done",
                    if status == "Done" { "true" } else { "false" },
                );
                if let Some(p) = self.task_project[n] {
                    item(
                        &mut out,
                        "project",
                        &quote(self.id(Kind::Project, p as usize)),
                    );
                }
                if self.count(Kind::Person) > 0 && r.chance(0.7) {
                    let who = r.below(self.count(Kind::Person) as u64) as usize;
                    item(&mut out, "assignee", &quote(self.id(Kind::Person, who)));
                }
                if r.chance(0.8) {
                    r.range(1, 2) as usize
                } else {
                    0
                }
            }
            Kind::Article => {
                item(&mut out, "topics", &list(&topics(&mut r)));
                item(
                    &mut out,
                    "published",
                    &quote(&self.date_after(&mut r, created)),
                );
                item(
                    &mut out,
                    "rating",
                    &format!("{}.{}", r.range(1, 4), r.below(10)),
                );
                let tags: Vec<String> = (0..r.range(0, 4))
                    .map(|_| self.common(&mut r).to_owned())
                    .collect();
                if !tags.is_empty() {
                    item(&mut out, "tags", &list(&tags));
                }
                let related = self.related(n);
                if !related.is_empty() {
                    item(&mut out, "related", &list(&related));
                }
                r.range(3, 8) as usize
            }
        };
        if paragraphs > 0 {
            out.push_str("\ncontent body {\n");
            if kind != Kind::Person {
                let heading = capitalise(&self.words(&mut r, 2, 5));
                out.push_str(&format!(
                    "  heading {{\n    level = 2\n    text = {}\n  }}\n",
                    quote(&heading)
                ));
            }
            let long = kind == Kind::Article;
            for _ in 0..paragraphs {
                let text = self.paragraph(&mut r, long);
                out.push_str(&format!(
                    "  paragraph {{\n    text = {}\n  }}\n",
                    quote(&text)
                ));
            }
            out.push_str("}\n");
        }
        out
    }

    /// The other articles in article `n`'s peer group. Every even group of three is linked.
    fn related(&self, n: usize) -> Vec<String> {
        let group = n / 3;
        if group % 2 == 1 {
            return Vec::new();
        }
        let end = (group * 3 + 3).min(self.count(Kind::Article));
        (group * 3..end)
            .filter(|&m| m != n)
            .map(|m| self.id(Kind::Article, m).to_owned())
            .collect()
    }

    fn status(&self, r: &mut Rng) -> &'static str {
        let mut roll = r.below(100);
        for (term, weight) in STATUSES {
            if roll < *weight {
                return term;
            }
            roll -= weight;
        }
        unreachable!("the status weights add up to 100")
    }

    fn common(&self, r: &mut Rng) -> &str {
        &self.words[r.skewed(VOCABULARY)]
    }

    fn words(&self, r: &mut Rng, lo: u64, hi: u64) -> String {
        let n = r.range(lo, hi);
        (0..n).map(|_| self.common(r)).collect::<Vec<_>>().join(" ")
    }

    fn title(&self, r: &mut Rng, kind: Kind) -> String {
        match kind {
            Kind::Person => format!(
                "{} {}",
                capitalise(self.common(r)),
                capitalise(self.common(r))
            ),
            _ => capitalise(&self.words(r, 3, 7)),
        }
    }

    /// A paragraph of sentences. Some words are bold and some paragraphs mention another note.
    fn paragraph(&self, r: &mut Rng, long: bool) -> String {
        let target = if long {
            r.range(40, 120)
        } else {
            r.range(12, 50)
        };
        let mut out = String::new();
        let mut written = 0;
        while written < target {
            let len = r.range(5, 14).min(target - written).max(1);
            let mut sentence = Vec::with_capacity(len as usize);
            for _ in 0..len {
                let w = self.common(r);
                if r.chance(0.02) {
                    sentence.push(format!("**{w}**"));
                } else {
                    sentence.push(w.to_owned());
                }
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&capitalise(&sentence.join(" ")));
            out.push('.');
            written += len;
        }
        if r.chance(0.1) {
            let other = r.below(self.ids.len() as u64) as usize;
            out.push_str(&format!(" See [[{}|{}]].", self.ids[other], self.common(r)));
        }
        out
    }

    /// A date from `created` to two years after it.
    fn date_after(&self, r: &mut Rng, created: u64) -> String {
        date((created + r.below(730 * 86_400_000)) / 86_400_000)
    }
}

/// One to three topic terms, each a root or a child.
fn topics(r: &mut Rng) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for _ in 0..r.range(1, 3) {
        let (root, children) = r.pick(TOPICS);
        let term = if r.chance(0.2) {
            (*root).to_owned()
        } else {
            format!("{root}/{}", r.pick(children))
        };
        if !out.contains(&term) {
            out.push(term);
        }
    }
    out
}

/// A made-up word of two to four syllables.
fn word(r: &mut Rng) -> String {
    const ONSETS: &[&str] = &[
        "b", "c", "d", "f", "g", "h", "j", "k", "l", "m", "n", "p", "r", "s", "t", "v", "w", "z",
        "br", "ch", "cl", "dr", "fl", "gr", "pl", "pr", "sh", "st", "th", "tr",
    ];
    const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ea", "ou", "y"];
    const CODAS: &[&str] = &["", "", "", "n", "r", "s", "l", "m", "nd", "st", "x"];
    let mut w = String::new();
    for _ in 0..r.range(2, 4) {
        w.push_str(r.pick(ONSETS));
        w.push_str(r.pick(VOWELS));
    }
    w.push_str(r.pick(CODAS));
    w
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn item(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(" = ");
    out.push_str(value);
    out.push('\n');
}

/// A WCL string. Generated text holds only letters, spaces, `.`, `*`, `[`, `]` and `|`.
fn quote(s: &str) -> String {
    format!("\"{s}\"")
}

fn list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| quote(s)).collect();
    format!("[{}]", quoted.join(", "))
}

/// `YYYY-MM-DD` for a count of days since 1970-01-01.
fn date(days: u64) -> String {
    let (y, m, d) = civil(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a count of milliseconds since the epoch.
fn datetime(ms: u64) -> String {
    let secs = ms / 1000;
    let day = secs % 86_400;
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        date(secs / 86_400),
        day / 3600,
        day / 60 % 60,
        day % 60
    )
}

/// The calendar date of a day count since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// What [`generate`] wrote.
#[derive(Debug)]
pub struct Generated {
    /// Notes of each type, in [`Kind::ALL`] order.
    pub counts: Vec<(&'static str, usize)>,
    /// Bytes of note files written.
    pub bytes: u64,
}

/// Write the vault `plan` describes into the empty folder `root`, on `threads` threads.
pub fn generate(root: &Path, plan: &Plan, threads: usize) -> Result<Generated> {
    if root.exists()
        && std::fs::read_dir(root)
            .with_context(|| format!("reading {}", root.display()))?
            .next()
            .is_some()
    {
        bail!("{} is not empty", root.display());
    }
    let vault = store::Vault::init(root).context("creating the vault")?;
    for (path, source) in DEFINITIONS {
        let text = store::canonical(source, path).with_context(|| format!("canonical {path}"))?;
        std::fs::write(vault.root().join(path), text).with_context(|| format!("writing {path}"))?;
    }
    for kind in Kind::ALL {
        std::fs::create_dir_all(vault.root().join("notes").join(kind.type_name()))?;
    }

    const CHUNK: usize = 1024;
    let next = AtomicUsize::new(0);
    let total = plan.ids.len();
    let bytes = std::thread::scope(|s| -> Result<u64> {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                s.spawn(|| -> Result<u64> {
                    let mut bytes = 0;
                    loop {
                        let start = next.fetch_add(CHUNK, Ordering::Relaxed);
                        if start >= total {
                            return Ok(bytes);
                        }
                        for i in start..(start + CHUNK).min(total) {
                            let path = plan.path(i);
                            let text = store::canonical(&plan.note(i), &path)
                                .with_context(|| format!("canonical {path}"))?;
                            std::fs::write(vault.root().join(&path), &text)
                                .with_context(|| format!("writing {path}"))?;
                            bytes += text.len() as u64;
                        }
                    }
                })
            })
            .collect();
        let mut bytes = 0;
        for w in workers {
            bytes += w.join().expect("a generator thread panicked")?;
        }
        Ok(bytes)
    })?;

    Ok(Generated {
        counts: Kind::ALL
            .iter()
            .map(|&k| (k.type_name(), plan.count(k)))
            .collect(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(20_454), "2026-01-01");
        assert_eq!(datetime(EPOCH_MS + 3_723_000), "2019-01-01T01:02:03Z");
    }

    #[test]
    fn every_topic_is_in_the_taxonomy_file() {
        let file = include_str!("../defs/taxonomies/topic.wcl");
        for (root, children) in TOPICS {
            assert!(file.contains(&format!("term \"{root}\"")), "{root}");
            for child in children {
                assert!(file.contains(&format!("term \"{child}\"")), "{child}");
            }
        }
    }
}
