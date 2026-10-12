//! The run in progress on this worker thread. Host functions are registered once and shared by
//! every thread, so what differs between runs (the author, the allowlist, the deadline, the log)
//! lives here for the length of one call.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use state::Author;
use wscript::HostError;

use crate::{Job, Limits};

pub struct Run {
    pub job: Job,
    pub limits: Limits,
    pub author: Author,
    pub deadline: Instant,
    pub log: String,
    /// Set once the deadline has passed inside a host call, so the fault it raised reads as a
    /// timeout rather than as whatever the host call was doing.
    pub timed_out: bool,
}

impl Run {
    /// The time left before the deadline, or the fault that ends the run once there is none.
    pub fn remaining(&mut self) -> Result<Duration, HostError> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            self.timed_out = true;
            Err(HostError::msg(format!(
                "the script ran longer than its {:?} time limit",
                self.limits.timeout
            )))
        } else {
            Ok(left)
        }
    }

    /// Append to the run log, up to the size limit.
    pub fn log(&mut self, text: &str, newline: bool) {
        const TRUNCATED: &str = "\n[log truncated]\n";
        let max = self.limits.max_log;
        if self.log.len() >= max {
            return;
        }
        let room = max - self.log.len();
        if text.len() + usize::from(newline) <= room {
            self.log.push_str(text);
            if newline {
                self.log.push('\n');
            }
        } else {
            let mut cut = room;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            self.log.push_str(&text[..cut]);
            self.log.push_str(TRUNCATED);
        }
    }
}

thread_local! {
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

/// Make `run` this thread's current run until [`finish`].
pub fn start(run: Run) {
    RUN.with(|r| *r.borrow_mut() = Some(run));
}

/// End this thread's current run and return it.
pub fn finish() -> Option<Run> {
    RUN.with(|r| r.borrow_mut().take())
}

/// Use the current run. Host functions only run inside one, so its absence is a host bug.
pub fn with<T>(f: impl FnOnce(&mut Run) -> Result<T, HostError>) -> Result<T, HostError> {
    RUN.with(|r| match r.borrow_mut().as_mut() {
        Some(run) => f(run),
        None => Err(HostError::msg(
            "no script run is in progress on this thread",
        )),
    })
}

/// Script output from `print` and `println`, which joins the run log.
pub fn print(text: &str, newline: bool) {
    let _ = with(|r| {
        r.log(text, newline);
        Ok(())
    });
}
