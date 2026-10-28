//! Helpers shared by the index tests. The writer and server tests load `hostile_pdf` and
//! `office` by path.

use std::time::Duration;

use index::Extracted;

pub mod hostile_pdf;
pub mod office;

/// What `extract` comes to once an extraction slot is free. Every test in this binary shares
/// one process, and with it the cap of [`index::MAX_RUNNING`] extractions at once, so a test
/// can find every slot taken: [`Extracted::Busy`] means try again.
pub fn when_free(extract: impl Fn() -> Extracted) -> Extracted {
    loop {
        match extract() {
            Extracted::Busy => std::thread::sleep(Duration::from_millis(10)),
            done => return done,
        }
    }
}
