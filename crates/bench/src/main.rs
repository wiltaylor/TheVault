//! `vault-bench`: the S6 benchmark (D31, D33, D39).
//!
//! It generates a vault of `--notes` notes from `--seed` (see [`bench::generate`]), times a full
//! `vault reindex` of it, starts `vault-server` on it, and times indexed queries, full-text
//! searches, note opens and saves over HTTP. Each measure is compared with its target in the PRD's
//! success criteria. The report goes to `<report-dir>/report.md` and `report.json`, and to stdout.
//! The exit code is 0 when every target was met and 2 when any was missed.
//!
//! The vault types define no handlers, so a timed save is a save excluding handlers. Requests are
//! sent one at a time from one client, so each timing is the latency of one request.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use bench::generate::{Kind, Plan, Spec, generate};
use bench::rng::Rng;
use bench::stats::{Summary, summarise};
use clap::Parser;
use reqwest::blocking::Client;
use reqwest::header;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "vault-bench",
    about = "Run the S6 benchmark on a generated vault"
)]
struct Args {
    /// Notes in the generated vault.
    #[arg(long, default_value_t = 1_000_000)]
    notes: usize,

    /// The generator seed. The same seed and note count always give the same vault.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Timed requests per measure.
    #[arg(long, default_value_t = 200)]
    samples: usize,

    /// Generator threads. Defaults to every core.
    #[arg(long)]
    threads: Option<usize>,

    /// The folder the vault is created in, as a fresh temporary folder.
    #[arg(long, default_value = "target/bench")]
    work_dir: PathBuf,

    /// The folder `report.md` and `report.json` are written to.
    #[arg(long, default_value = "target/bench")]
    report_dir: PathBuf,

    /// Keep the generated vault instead of deleting it at the end.
    #[arg(long)]
    keep: bool,

    /// The `vault-server` binary. Defaults to the one beside this binary.
    #[arg(long)]
    server_bin: Option<PathBuf>,

    /// The `vault` CLI binary. Defaults to the one beside this binary.
    #[arg(long)]
    cli_bin: Option<PathBuf>,
}

/// One measure and its target from the PRD's success criteria.
struct Target {
    key: &'static str,
    name: &'static str,
    /// The measured value must be under this many milliseconds.
    limit_ms: f64,
    target: &'static str,
}

const QUERY: Target = Target {
    key: "query_p95",
    name: "Query on indexed fields, p95",
    limit_ms: 500.0,
    target: "under 500 ms",
};
const SEARCH: Target = Target {
    key: "search_p95",
    name: "Full-text search, p95",
    limit_ms: 1_000.0,
    target: "under 1 s",
};
const SAVE: Target = Target {
    key: "save_p95",
    name: "Save, excluding handlers, p95",
    limit_ms: 200.0,
    target: "under 200 ms",
};
const OPEN: Target = Target {
    key: "open_p95",
    name: "Open a note, p95",
    limit_ms: 300.0,
    target: "under 300 ms",
};
const REINDEX: Target = Target {
    key: "reindex",
    name: "Full `vault reindex`",
    limit_ms: 30.0 * 60_000.0,
    target: "under 30 minutes",
};

/// The password the generated vault's `state.db` is given. Nothing logs in with it.
const PASSWORD: &str = "vault-bench";

/// Untimed requests sent before each measure.
const WARM_UP: usize = 5;

fn main() {
    match run(Args::parse()) {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(e) => {
            eprintln!("vault-bench: {e:#}");
            std::process::exit(1);
        }
    }
}

/// Run the benchmark and write the report. True when every target was met.
fn run(args: Args) -> Result<bool> {
    let server_bin = sibling(args.server_bin.as_deref(), "vault-server")?;
    let cli_bin = sibling(args.cli_bin.as_deref(), "vault")?;
    let threads = args.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });
    ensure!(args.samples > 0, "--samples must be at least 1");

    std::fs::create_dir_all(&args.work_dir)
        .with_context(|| format!("creating {}", args.work_dir.display()))?;
    let dir = tempfile::Builder::new()
        .prefix("vault-")
        .tempdir_in(&args.work_dir)
        .context("creating the vault folder")?;
    let root = dir.path().canonicalize()?;

    let spec = Spec {
        notes: args.notes,
        seed: args.seed,
    };
    progress(&format!(
        "generating {} notes with seed {} in {}",
        spec.notes,
        spec.seed,
        root.display()
    ));
    let started = Instant::now();
    let plan = Plan::new(spec);
    let generated = generate(&root, &plan, threads)?;
    let generate_time = started.elapsed();
    progress(&format!("generated in {}", secs(generate_time)));

    let token = credentials(&root)?;

    progress("timing vault reindex");
    let reindex_time = reindex(&cli_bin, &root, spec.notes)?;
    progress(&format!("reindexed in {}", secs(reindex_time)));

    progress("starting vault-server");
    let started = Instant::now();
    let server = Server::start(&server_bin, &root)?;
    let startup_time = started.elapsed();
    progress(&format!("server ready in {}", secs(startup_time)));

    let client = Client::builder()
        .default_headers({
            let mut h = header::HeaderMap::new();
            h.insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().context("token header")?,
            );
            h
        })
        .timeout(Duration::from_secs(600))
        .build()?;
    let bench = Bench {
        client,
        base: server.base.clone(),
        plan: &plan,
        samples: args.samples,
    };

    progress("timing queries");
    let query = bench.queries()?;
    progress("timing full-text search");
    let search = bench.searches()?;
    progress("timing note opens");
    let open = bench.opens()?;
    progress("timing saves");
    let save = bench.saves()?;
    drop(server);

    let rows = [
        (&QUERY, query.0.p95, Some(&query.0)),
        (&SEARCH, search.0.p95, Some(&search.0)),
        (&SAVE, save.p95, Some(&save)),
        (&OPEN, open.p95, Some(&open)),
        (&REINDEX, reindex_time.as_secs_f64() * 1000.0, None),
    ];
    let all_met = rows.iter().all(|(t, v, _)| *v < t.limit_ms);

    let machine = machine(&root);
    let context = RunInfo {
        args: &args,
        threads,
        generated: &generated,
        generate_time,
        startup_time,
        machine: &machine,
        queries: &query.1,
        searches: &search.1,
    };
    let markdown = markdown(&rows, &context);
    let json = report_json(&rows, &context, all_met);
    std::fs::create_dir_all(&args.report_dir)?;
    let md_path = args.report_dir.join("report.md");
    let json_path = args.report_dir.join("report.json");
    std::fs::write(&md_path, &markdown)?;
    std::fs::write(&json_path, serde_json::to_string_pretty(&json)? + "\n")?;
    println!("{markdown}");
    progress(&format!(
        "report written to {} and {}",
        md_path.display(),
        json_path.display()
    ));

    if args.keep {
        let kept = dir.keep();
        progress(&format!("kept the vault at {}", kept.display()));
    }
    Ok(all_met)
}

fn progress(message: &str) {
    eprintln!("vault-bench: {message}");
}

fn secs(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

/// `given`, or else the binary `name` in the folder this binary is in.
fn sibling(given: Option<&Path>, name: &str) -> Result<PathBuf> {
    let path = match given {
        Some(p) => p.to_owned(),
        None => std::env::current_exe()?
            .parent()
            .context("this binary has no folder")?
            .join(name),
    };
    ensure!(
        path.is_file(),
        "{} not found: build it, or name it with --{}-bin",
        path.display(),
        if name == "vault" { "cli" } else { "server" }
    );
    Ok(path)
}

/// Set the login password and create an `admin` token, returning the token secret.
fn credentials(root: &Path) -> Result<String> {
    let state = state::State::open(&root.join(".vault/state.db"), None).context("state.db")?;
    state.set_password(PASSWORD)?;
    let (_, secret) = state.create_token("bench", state::Scope::Admin)?;
    Ok(secret)
}

/// Time `vault reindex` and check that it indexed every note with no failures.
fn reindex(cli: &Path, root: &Path, notes: usize) -> Result<Duration> {
    let started = Instant::now();
    let out = Command::new(cli)
        .arg("--vault")
        .arg(root)
        .arg("reindex")
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("running {}", cli.display()))?;
    let elapsed = started.elapsed();
    let stdout = String::from_utf8_lossy(&out.stdout);
    ensure!(
        out.status.success(),
        "vault reindex failed: {}",
        truncate(&stdout, 4000)
    );
    let report: Value = serde_json::from_str(stdout.trim())
        .with_context(|| format!("vault reindex printed {}", truncate(&stdout, 400)))?;
    ensure!(
        report["indexed"].as_u64() == Some(notes as u64),
        "vault reindex indexed {} of {notes} notes",
        report["indexed"]
    );
    Ok(elapsed)
}

fn truncate(s: &str, n: usize) -> &str {
    &s[..s.floor_char_boundary(n)]
}

/// A running `vault-server`, killed when dropped.
struct Server {
    child: Child,
    base: String,
}

impl Server {
    fn start(binary: &Path, root: &Path) -> Result<Server> {
        let mut child = Command::new(binary)
            .env_remove("VAULT_SECRET_KEY_FILE")
            .arg("--vault")
            .arg(root)
            .arg("--listen")
            .arg("127.0.0.1:0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("starting {}", binary.display()))?;
        let stdout = child.stdout.take().context("no stdout pipe")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let read = reader.read_line(&mut line);
        if read.is_err() || line.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("vault-server exited before reporting its address");
        }
        // Keep draining stdout so the server never blocks on a full pipe.
        std::thread::spawn(move || std::io::copy(&mut reader, &mut std::io::sink()));
        let value: Value = serde_json::from_str(line.trim())
            .with_context(|| format!("vault-server printed {line:?}"))?;
        let listen = value["listen"]
            .as_str()
            .with_context(|| format!("no `listen` in {line:?}"))?;
        Ok(Server {
            child,
            base: format!("http://{listen}"),
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Bench<'a> {
    client: Client,
    base: String,
    plan: &'a Plan,
    samples: usize,
}

impl Bench<'_> {
    /// Run `request` [`WARM_UP`] times untimed and then `samples` times timed. `request` gets the
    /// sample number and returns the timed part.
    fn measure(&self, mut request: impl FnMut(usize) -> Result<Duration>) -> Result<Summary> {
        for i in 0..WARM_UP {
            request(i)?;
        }
        let mut times = Vec::with_capacity(self.samples);
        for i in 0..self.samples {
            times.push(request(WARM_UP + i)?);
        }
        Ok(summarise(&times))
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<(Duration, Value)> {
        let started = Instant::now();
        let res = self
            .client
            .get(format!("{}{path}", self.base))
            .query(query)
            .send()?;
        let status = res.status();
        let body = res.text()?;
        let elapsed = started.elapsed();
        ensure!(
            status.is_success(),
            "GET {path} {query:?}: {status} {}",
            truncate(&body, 400)
        );
        Ok((elapsed, serde_json::from_str(&body)?))
    }

    /// The queries timed, as text. Some name a note, so they vary by sample.
    fn query_text(&self, i: usize) -> String {
        let mut r = Rng::for_item(self.plan.spec.seed ^ 0x0E_E1, i as u64);
        let mut some = |kind: Kind| -> String {
            let ids = self.plan.ids(kind);
            if ids.is_empty() {
                String::new()
            } else {
                ids[r.below(ids.len() as u64) as usize].clone()
            }
        };
        let queries = [
            "from task where status == \"Active/In progress\" sort due asc".to_owned(),
            "from task where under(status, \"Active\") && due < \"2026-01-01\" sort due asc"
                .to_owned(),
            "from task where priority == \"high\" && !done sort updated desc".to_owned(),
            format!(
                "from task where project == \"{}\" sort due asc",
                some(Kind::Project)
            ),
            format!(
                "from project where owner == \"{}\" sort title asc",
                some(Kind::Person)
            ),
            "from article where has(topics, \"Science/Physics\") sort published desc".to_owned(),
            "from article where under(topics, \"Tech\") && rating >= 4.0 sort rating desc"
                .to_owned(),
            "from * sort updated desc".to_owned(),
            "from task where estimate > 8 && !done group by status".to_owned(),
        ];
        queries[i % queries.len()].clone()
    }

    /// The query timings, and each query shape's p95 for the report.
    fn queries(&self) -> Result<(Summary, Vec<(String, Summary)>)> {
        let all = self.measure(|i| {
            let q = self.query_text(i);
            self.get("/api/query", &[("q", &q)]).map(|(t, _)| t)
        })?;
        let shapes = self.per_shape(9, |i| self.query_text(i), "/api/query")?;
        Ok((all, shapes))
    }

    /// The search terms timed: single words from common to rare, and pairs.
    fn search_text(&self, i: usize) -> String {
        let p = self.plan;
        let terms = [
            p.word(0).to_owned(),
            p.word(5).to_owned(),
            p.word(50).to_owned(),
            p.word(500).to_owned(),
            p.word(5_000).to_owned(),
            p.word(15_000).to_owned(),
            format!("{} {}", p.word(1), p.word(2)),
            format!("{} {}", p.word(100), p.word(3_000)),
        ];
        terms[i % terms.len()].clone()
    }

    fn searches(&self) -> Result<(Summary, Vec<(String, Summary)>)> {
        let all = self.measure(|i| {
            let q = self.search_text(i);
            self.get("/api/search", &[("q", &q)]).map(|(t, _)| t)
        })?;
        let shapes = self.per_shape(8, |i| self.search_text(i), "/api/search")?;
        Ok((all, shapes))
    }

    /// Time each of `shapes` request texts on its own, a fifth of `samples` times, so the report
    /// can show which is slow.
    fn per_shape(
        &self,
        shapes: usize,
        text: impl Fn(usize) -> String,
        path: &str,
    ) -> Result<Vec<(String, Summary)>> {
        let runs = (self.samples / 5).max(1);
        let mut out = Vec::new();
        for s in 0..shapes {
            let mut times = Vec::with_capacity(runs);
            let mut shown = String::new();
            for k in 0..runs {
                let q = text(s + k * shapes);
                let (t, _) = self.get(path, &[("q", &q)])?;
                times.push(t);
                shown = q;
            }
            out.push((shown, summarise(&times)));
        }
        Ok(out)
    }

    /// A note picked at random, from every type in proportion.
    fn some_note(&self, r: &mut Rng) -> String {
        let n = self.plan.spec.notes as u64;
        let mut i = r.below(n) as usize;
        for kind in Kind::ALL {
            let count = self.plan.count(kind);
            if i < count {
                return self.plan.id(kind, i).to_owned();
            }
            i -= count;
        }
        unreachable!("the plan's counts add up to its notes")
    }

    fn opens(&self) -> Result<Summary> {
        self.measure(|i| {
            let mut r = Rng::for_item(self.plan.spec.seed ^ 0x09E7, i as u64);
            let id = self.some_note(&mut r);
            let (t, body) = self.get(&format!("/api/notes/{id}"), &[])?;
            ensure!(
                body["id"] == id.as_str(),
                "opened {id} but got {}",
                body["id"]
            );
            Ok(t)
        })
    }

    /// Change the title of a random note. Reading the note and its version is not timed.
    fn saves(&self) -> Result<Summary> {
        self.measure(|i| {
            let mut r = Rng::for_item(self.plan.spec.seed ^ 0x5A7E, i as u64);
            let id = self.some_note(&mut r);
            let url = format!("{}/api/notes/{id}", self.base);
            let res = self
                .client
                .get(&url)
                .header(header::ACCEPT, "text/plain")
                .send()?
                .error_for_status()?;
            let etag = res
                .headers()
                .get(header::ETAG)
                .context("no ETag on a note")?
                .clone();
            let wcl = res.text()?;
            let (first, rest) = wcl.split_once('\n').context("an empty note")?;
            ensure!(first.starts_with("title"), "the note starts with {first:?}");
            let edited = format!(
                "{} (edit {i})\"\n{rest}",
                first.trim_end().trim_end_matches('"')
            );

            let started = Instant::now();
            let res = self
                .client
                .put(&url)
                .header(header::CONTENT_TYPE, "text/plain")
                .header(header::IF_MATCH, etag)
                .body(edited)
                .send()?;
            let status = res.status();
            let body = res.text()?;
            let elapsed = started.elapsed();
            ensure!(
                status.is_success(),
                "PUT {id}: {status} {}",
                truncate(&body, 400)
            );
            Ok(elapsed)
        })
    }
}

/// What the report says about the run besides the measures.
struct RunInfo<'a> {
    args: &'a Args,
    threads: usize,
    generated: &'a bench::generate::Generated,
    generate_time: Duration,
    startup_time: Duration,
    machine: &'a Machine,
    queries: &'a [(String, Summary)],
    searches: &'a [(String, Summary)],
}

struct Machine {
    cpu: String,
    memory: String,
    filesystem: String,
}

impl Machine {
    /// Whether this is the D39 reference machine's processor.
    fn is_reference(&self) -> bool {
        self.cpu.contains("Ryzen 9 9950X")
    }
}

/// The processor, memory and the filesystem the vault is on, from `/proc`.
fn machine(vault: &Path) -> Machine {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let cpuinfo = read("/proc/cpuinfo");
    let cpu = cpuinfo
        .lines()
        .find_map(|l| l.strip_prefix("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    let threads = cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count();
    let memory = read("/proc/meminfo")
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.trim().strip_suffix(" kB")?.trim().parse::<u64>().ok())
        .map(|kb| format!("{:.0} GiB", kb as f64 / (1024.0 * 1024.0)))
        .unwrap_or_else(|| "unknown".into());
    // The mount whose mount point is the longest prefix of the vault path.
    let filesystem = read("/proc/self/mountinfo")
        .lines()
        .filter_map(|l| {
            let (left, right) = l.split_once(" - ")?;
            let point = left.split(' ').nth(4)?;
            let fs = right.split(' ').next()?;
            vault
                .starts_with(point)
                .then(|| (point.len(), fs.to_owned()))
        })
        .max()
        .map(|(_, fs)| fs)
        .unwrap_or_else(|| "unknown".into());
    Machine {
        cpu: format!("{cpu} ({threads} threads)"),
        memory,
        filesystem,
    }
}

fn verdict(value: f64, t: &Target) -> &'static str {
    if value < t.limit_ms { "pass" } else { "FAIL" }
}

fn shown(t: &Target, value_ms: f64) -> String {
    if t.key == "reindex" {
        format!("{:.1} s", value_ms / 1000.0)
    } else {
        format!("{value_ms:.1} ms")
    }
}

type Row<'a> = (&'a Target, f64, Option<&'a Summary>);

fn markdown(rows: &[Row], c: &RunInfo) -> String {
    let mut out = String::new();
    out.push_str("# S6 benchmark report\n\n");
    out.push_str(&format!(
        "{} notes, seed {}, {} timed requests per measure after {WARM_UP} untimed.\n\n",
        c.args.notes, c.args.seed, c.args.samples
    ));
    out.push_str("| Measure | Target | Measured | p50 | max | Result |\n");
    out.push_str("|---------|--------|----------|-----|-----|--------|\n");
    for (t, value, summary) in rows {
        let (p50, max) = match summary {
            Some(s) => (format!("{:.1} ms", s.p50), format!("{:.1} ms", s.max)),
            None => ("".into(), "".into()),
        };
        out.push_str(&format!(
            "| {} | {} | {} | {p50} | {max} | {} |\n",
            t.name,
            t.target,
            shown(t, *value),
            verdict(*value, t)
        ));
    }
    out.push_str(&format!(
        "\n## Machine\n\n- Processor: {}\n- Memory: {}\n- Vault filesystem: {}\n- Reference machine (D39): {}\n",
        c.machine.cpu,
        c.machine.memory,
        c.machine.filesystem,
        if c.machine.is_reference() { "yes" } else { "no, so these results do not count against the targets" }
    ));
    out.push_str("\n## Vault\n\n");
    for (name, n) in &c.generated.counts {
        out.push_str(&format!("- {name}: {n}\n"));
    }
    out.push_str(&format!(
        "- Note files: {:.1} MiB\n- Generated in {} on {} threads, written through `store`\n- `vault-server` start, which rebuilds the index: {}\n",
        c.generated.bytes as f64 / (1024.0 * 1024.0),
        secs(c.generate_time),
        c.threads,
        secs(c.startup_time),
    ));
    for (title, shapes) in [("Queries", c.queries), ("Searches", c.searches)] {
        out.push_str(&format!(
            "\n## {title}\n\n| Text | p50 | p95 | max |\n|------|-----|-----|-----|\n"
        ));
        for (text, s) in shapes {
            out.push_str(&format!(
                "| `{}` | {:.1} ms | {:.1} ms | {:.1} ms |\n",
                text.replace('|', "\\|"),
                s.p50,
                s.p95,
                s.max
            ));
        }
    }
    out
}

fn summary_json(s: &Summary) -> Value {
    json!({ "samples": s.samples, "p50_ms": s.p50, "p95_ms": s.p95, "max_ms": s.max })
}

fn report_json(rows: &[Row], c: &RunInfo, all_met: bool) -> Value {
    let measures: Vec<Value> = rows
        .iter()
        .map(|(t, value, summary)| {
            json!({
                "key": t.key,
                "name": t.name,
                "target": t.target,
                "limit_ms": t.limit_ms,
                "value_ms": value,
                "pass": *value < t.limit_ms,
                "summary": summary.map(summary_json),
            })
        })
        .collect();
    let shapes = |list: &[(String, Summary)]| -> Vec<Value> {
        list.iter()
            .map(|(text, s)| json!({ "text": text, "summary": summary_json(s) }))
            .collect()
    };
    json!({
        "notes": c.args.notes,
        "seed": c.args.seed,
        "samples": c.args.samples,
        "all_met": all_met,
        "measures": measures,
        "machine": {
            "cpu": c.machine.cpu,
            "memory": c.machine.memory,
            "filesystem": c.machine.filesystem,
            "reference": c.machine.is_reference(),
        },
        "vault": {
            "counts": c.generated.counts.iter().map(|(k, n)| json!({ "type": k, "notes": n })).collect::<Vec<_>>(),
            "bytes": c.generated.bytes,
            "generate_s": c.generate_time.as_secs_f64(),
            "generator_threads": c.threads,
            "server_start_s": c.startup_time.as_secs_f64(),
        },
        "queries": shapes(c.queries),
        "searches": shapes(c.searches),
    })
}
