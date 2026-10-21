//! How far a sync round has got, for the desktop setup page's first-sync bar and the sync popover
//! (ticket 144; D19, D20, D29).
//!
//! The engine updates one [`SyncProgress`] as a round runs: its [`Stage`], the feed entries pulled
//! out of those there are, the asset files fetched out of those to fetch, and their bytes as they
//! arrive. Anyone holding a clone reads a [`Progress`] snapshot at any time, from another thread.
//!
//! **Totals.** The change feed answers each page with `remaining`, how many entries come after
//! it, so the notes total is what the round has pulled, plus the page, plus `remaining`. An older
//! server sends only `latest`, the newest `seq`, and `latest - next` stands in for `remaining`:
//! an upper bound, since a key that changes again takes a new `seq` and leaves a gap. The files
//! total comes from `GET /api/assets`, which the round reads before it pulls: every asset the
//! server lists that the device does not hold, with its size.
//!
//! **Time left.** The engine keeps a sample of the counts every quarter second or so, and the
//! rates are measured over the last [`WINDOW`]: notes a second, and asset bytes a second. Time
//! left is notes left over the notes rate, plus bytes left over the bytes rate. While the round
//! pulls notes and no asset byte has arrived yet, the bytes rate is the feed's own, bytes of feed
//! pages a second, the best guess of how fast this network moves data. The sum is rounded to the
//! nearest second. There is no estimate until a rate has been measured for at least [`MIN_SPAN`].
//!
//! Every count and total only grows during a round; [`SyncProgress::begin`] starts the next one
//! from zero.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How far back the transfer rates are measured.
pub const WINDOW: Duration = Duration::from_secs(10);

/// How long a rate must be measured over before it gives an estimate.
pub const MIN_SPAN: Duration = Duration::from_millis(1000);

/// How often at most a sample is kept.
const SAMPLE_EVERY: Duration = Duration::from_millis(250);

/// A round that pulls more feed entries than this, or fetches more bytes than [`LARGE_BYTES`],
/// shows its progress in the sync popover.
pub const LARGE_NOTES: u64 = 200;

/// See [`LARGE_NOTES`]: 20 MB.
pub const LARGE_BYTES: u64 = 20 * 1024 * 1024;

/// Where a round is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// No round has run since the engine was made.
    Idle,
    /// Asking the server its versions.
    Handshake,
    /// Sending the device's assets, notes and notifications.
    Pushing,
    /// Reading the change feed.
    Pulling,
    /// Downloading asset files.
    Fetching,
    /// The round finished.
    Done,
    /// The round failed.
    Failed,
}

impl Stage {
    /// The stage's name in the status JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Idle => "idle",
            Stage::Handshake => "handshake",
            Stage::Pushing => "pushing",
            Stage::Pulling => "pulling",
            Stage::Fetching => "fetching",
            Stage::Done => "done",
            Stage::Failed => "failed",
        }
    }

    /// Whether a round is running.
    pub fn running(self) -> bool {
        matches!(
            self,
            Stage::Handshake | Stage::Pushing | Stage::Pulling | Stage::Fetching
        )
    }
}

/// A snapshot of a round's progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    /// Feed entries (notes and definition files) pulled this round.
    pub notes_done: u64,
    /// Feed entries this round pulls, as far as the server has said so far.
    pub notes_total: u64,
    /// Asset files fetched this round.
    pub files_done: u64,
    /// Asset files this round fetches.
    pub files_total: u64,
    /// Asset bytes received this round, counted as they arrive.
    pub bytes_done: u64,
    /// The sizes of the asset files this round fetches, added up.
    pub bytes_total: u64,
    /// About how many seconds the round has left; `None` until there is a rate to go by.
    pub eta_secs: Option<u64>,
    /// How long the round has run, or ran, in milliseconds.
    pub elapsed_ms: u64,
}

impl Progress {
    fn idle() -> Progress {
        Progress {
            stage: Stage::Idle,
            notes_done: 0,
            notes_total: 0,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            eta_secs: None,
            elapsed_ms: 0,
        }
    }

    /// Whether the round pulls or downloads enough to show its progress in the sync popover:
    /// more than [`LARGE_NOTES`] feed entries or [`LARGE_BYTES`] of assets.
    pub fn is_large(&self) -> bool {
        self.notes_total > LARGE_NOTES || self.bytes_total > LARGE_BYTES
    }
}

/// One sample of the counts.
#[derive(Debug, Clone, Copy)]
struct Sample {
    at: Instant,
    notes: u64,
    bytes: u64,
    feed_bytes: u64,
}

#[derive(Debug)]
struct Inner {
    progress: Progress,
    started: Instant,
    ended: Option<Instant>,
    /// Bytes of feed pages read this round.
    feed_bytes: u64,
    samples: VecDeque<Sample>,
}

impl Inner {
    fn sample(&mut self, now: Instant, force: bool) {
        if !force
            && self
                .samples
                .back()
                .is_some_and(|s| now.duration_since(s.at) < SAMPLE_EVERY)
        {
            return;
        }
        self.samples.push_back(Sample {
            at: now,
            notes: self.progress.notes_done,
            bytes: self.progress.bytes_done,
            feed_bytes: self.feed_bytes,
        });
        while self
            .samples
            .front()
            .is_some_and(|s| now.duration_since(s.at) > WINDOW)
            && self.samples.len() > 2
        {
            self.samples.pop_front();
        }
    }

    /// Time left at `now`, from the rates over the window.
    fn eta(&self, now: Instant) -> Option<u64> {
        let p = &self.progress;
        if !p.stage.running() {
            return (p.stage == Stage::Done).then_some(0);
        }
        let notes_left = p.notes_total.saturating_sub(p.notes_done);
        let bytes_left = p.bytes_total.saturating_sub(p.bytes_done);
        if notes_left == 0 && bytes_left == 0 {
            return (p.notes_total > 0 || p.bytes_total > 0).then_some(0);
        }
        let first = self.samples.front()?;
        let span = now.duration_since(first.at);
        if span < MIN_SPAN {
            return None;
        }
        let secs = span.as_secs_f64();
        let rate = |done: u64, then: u64| {
            let moved = done.saturating_sub(then);
            (moved > 0).then(|| moved as f64 / secs)
        };
        let mut left = 0.0;
        if notes_left > 0 {
            left += notes_left as f64 / rate(p.notes_done, first.notes)?;
        }
        if bytes_left > 0 {
            let bytes_rate = rate(p.bytes_done, first.bytes)
                .or_else(|| rate(self.feed_bytes, first.feed_bytes))?;
            left += bytes_left as f64 / bytes_rate;
        }
        Some(left.round() as u64)
    }
}

/// A round's progress, shared between the engine that updates it and whoever shows it.
#[derive(Debug, Clone)]
pub struct SyncProgress(Arc<Mutex<Inner>>);

impl Default for SyncProgress {
    fn default() -> Self {
        SyncProgress::new()
    }
}

impl SyncProgress {
    pub fn new() -> SyncProgress {
        SyncProgress(Arc::new(Mutex::new(Inner {
            progress: Progress::idle(),
            started: Instant::now(),
            ended: None,
            feed_bytes: 0,
            samples: VecDeque::new(),
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Where the round stands now.
    pub fn snapshot(&self) -> Progress {
        let inner = self.lock();
        let now = Instant::now();
        let mut out = inner.progress.clone();
        out.eta_secs = inner.eta(now);
        out.elapsed_ms = inner
            .ended
            .unwrap_or(now)
            .duration_since(inner.started)
            .as_millis() as u64;
        out
    }

    /// A round starts: everything from zero, at the handshake.
    pub(crate) fn begin(&self) {
        let mut inner = self.lock();
        inner.progress = Progress::idle();
        inner.progress.stage = Stage::Handshake;
        inner.started = Instant::now();
        inner.ended = None;
        inner.feed_bytes = 0;
        inner.samples.clear();
    }

    pub(crate) fn stage(&self, stage: Stage) {
        let mut inner = self.lock();
        inner.progress.stage = stage;
        // The rates start again with each stage: pushing says nothing of how fast a pull goes.
        inner.samples.clear();
        inner.sample(Instant::now(), true);
    }

    /// The round ends, finished or failed.
    pub(crate) fn finish(&self, ok: bool) {
        let mut inner = self.lock();
        inner.progress.stage = if ok { Stage::Done } else { Stage::Failed };
        inner.ended = Some(Instant::now());
    }

    /// A feed page of `entries` entries and `bytes` bytes was read, and the server says
    /// `remaining` more come after it. A total only grows.
    pub(crate) fn feed_page(&self, entries: u64, bytes: u64, remaining: u64) {
        let mut inner = self.lock();
        let p = &mut inner.progress;
        p.notes_total = p.notes_total.max(p.notes_done + entries + remaining);
        inner.feed_bytes += bytes;
        inner.sample(Instant::now(), false);
    }

    /// `n` feed entries were applied.
    pub(crate) fn notes_applied(&self, n: u64) {
        let mut inner = self.lock();
        let p = &mut inner.progress;
        p.notes_done += n;
        p.notes_total = p.notes_total.max(p.notes_done);
        inner.sample(Instant::now(), false);
    }

    /// The round fetches `files` more asset files of `bytes` bytes in all.
    pub(crate) fn plan_files(&self, files: u64, bytes: u64) {
        let mut inner = self.lock();
        let p = &mut inner.progress;
        p.files_total = p.files_total.max(p.files_done + files);
        p.bytes_total = p.bytes_total.max(p.bytes_done + bytes);
    }

    /// `n` asset bytes arrived.
    pub(crate) fn bytes(&self, n: u64) {
        let mut inner = self.lock();
        let p = &mut inner.progress;
        p.bytes_done += n;
        p.bytes_total = p.bytes_total.max(p.bytes_done);
        inner.sample(Instant::now(), false);
    }

    /// One asset file was fetched.
    pub(crate) fn file_fetched(&self) {
        let mut inner = self.lock();
        let p = &mut inner.progress;
        p.files_done += 1;
        p.files_total = p.files_total.max(p.files_done);
    }

    /// Move everything the round recorded `by` into the past, for the tests of the estimate.
    #[cfg(test)]
    fn age(&self, by: Duration) {
        let mut inner = self.lock();
        for s in inner.samples.iter_mut() {
            s.at -= by;
        }
        inner.started -= by;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tracker_is_idle_with_nothing_counted() {
        let p = SyncProgress::new().snapshot();
        assert_eq!(p.stage, Stage::Idle);
        assert_eq!((p.notes_total, p.files_total, p.bytes_total), (0, 0, 0));
        assert_eq!(p.eta_secs, None);
        assert!(!p.is_large());
    }

    #[test]
    fn totals_only_grow_and_counts_never_pass_them() {
        let t = SyncProgress::new();
        t.begin();
        t.stage(Stage::Pulling);
        t.feed_page(500, 1000, 700);
        assert_eq!(t.snapshot().notes_total, 1200);
        t.notes_applied(500);
        // The server now says fewer remain, as when a key moved: the total stays.
        t.feed_page(500, 1000, 100);
        assert_eq!(t.snapshot().notes_total, 1200);
        t.notes_applied(500);
        t.feed_page(300, 600, 0);
        t.notes_applied(300);
        let p = t.snapshot();
        assert_eq!((p.notes_done, p.notes_total), (1300, 1300));
        assert!(p.is_large());
    }

    #[test]
    fn the_estimate_is_notes_left_over_the_notes_rate_plus_bytes_left_over_the_bytes_rate() {
        let t = SyncProgress::new();
        t.begin();
        t.stage(Stage::Fetching);
        t.plan_files(4, 40_000_000);
        assert_eq!(t.snapshot().eta_secs, None, "no rate yet");
        // 10 MB in the first two seconds: 5 MB/s, 30 MB to go, so 6 s.
        t.age(Duration::from_secs(2));
        t.bytes(10_000_000);
        assert_eq!(t.snapshot().eta_secs, Some(6));

        let t = SyncProgress::new();
        t.begin();
        t.plan_files(1, 10_000);
        t.stage(Stage::Pulling);
        t.feed_page(100, 50_000, 300);
        t.age(Duration::from_secs(1));
        t.notes_applied(100);
        // 100 notes and 50 KB of feed a second: 300 notes and 10 KB left is 3 s and 0.2 s.
        assert_eq!(t.snapshot().eta_secs, Some(3));
        t.finish(true);
        assert_eq!(t.snapshot().eta_secs, Some(0));
        assert_eq!(t.snapshot().stage, Stage::Done);
    }
}
