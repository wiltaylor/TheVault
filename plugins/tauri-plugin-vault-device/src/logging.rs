//! Where the core's logs go (ticket 88). On a phone, `log` records go to logcat under the tag
//! `TheVault`, and so does every line written to stderr, which is how the crates the core embeds
//! report a problem (`eprintln!`): an Android app's stderr goes nowhere otherwise. Elsewhere, as
//! in the JVM and Rust tests, records go to stderr.
//!
//! [`init`] runs once per process, from [`crate::core::open`]; calling it again does nothing.

use std::sync::Once;

static INIT: Once = Once::new();

/// Send the process's logs where they can be read, once.
pub fn init() {
    INIT.call_once(|| {
        #[cfg(target_os = "android")]
        {
            android_logger::init_once(
                android_logger::Config::default()
                    .with_tag("TheVault")
                    .with_max_level(log::LevelFilter::Info),
            );
            android::forward_stderr();
        }
        #[cfg(not(target_os = "android"))]
        {
            if log::set_logger(&Stderr).is_ok() {
                log::set_max_level(log::LevelFilter::Info);
            }
        }
    });
}

/// Records to stderr, off the phone.
#[cfg(not(target_os = "android"))]
struct Stderr;

#[cfg(not(target_os = "android"))]
impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{} {}: {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

#[cfg(target_os = "android")]
mod android {
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use std::os::fd::FromRawFd;

    /// Point stderr at a pipe and log each line read from it at `warn`.
    pub fn forward_stderr() {
        let mut fds = [0; 2];
        // SAFETY: `fds` holds two ints for `pipe` to fill; the read end is owned by the File
        // below from then on, and the write end is closed once stderr is a copy of it.
        let read = unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return;
            }
            if libc::dup2(fds[1], libc::STDERR_FILENO) < 0 {
                libc::close(fds[0]);
                libc::close(fds[1]);
                return;
            }
            libc::close(fds[1]);
            File::from_raw_fd(fds[0])
        };
        let started = std::thread::Builder::new()
            .name("vault-stderr".into())
            .spawn(move || {
                for line in BufReader::new(read).lines().map_while(Result::ok) {
                    log::warn!(target: "stderr", "{line}");
                }
            });
        if let Err(e) = started {
            log::warn!("stderr is not forwarded to logcat: {e}");
        }
    }
}
