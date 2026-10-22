//! An incremental server-sent events parser. Both transports feed it the body's byte chunks as
//! they arrive, wherever the chunks happen to split, and get back each complete frame.
//!
//! It follows the event-stream format: lines end in `\n`, `\r\n` or `\r`; a blank line ends a
//! frame; `event:` names it and each `data:` line adds a line of data; one space after the colon
//! is dropped; lines starting `:` are comments; `id:`, `retry:` and unknown fields are ignored.
//! A frame with no `data:` line is not dispatched. Lines are decoded as UTF-8 only once whole, so
//! a character split between chunks survives.

/// One dispatched frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// The `event:` name, or `None` for the default `message`.
    pub event: Option<String>,
    /// The `data:` lines joined with `\n`.
    pub data: String,
}

/// The parser's state between chunks.
#[derive(Debug, Default)]
pub struct SseParser {
    /// Bytes of a line not yet ended.
    line: Vec<u8>,
    /// The last chunk ended in `\r`, so a `\n` starting the next one ends nothing.
    after_cr: bool,
    /// At the very start, where a byte order mark is skipped.
    started: bool,
    event: Option<String>,
    data: Option<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take `chunk` and return every frame it completes.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        let mut out = Vec::new();
        for &b in chunk {
            if self.after_cr {
                self.after_cr = false;
                if b == b'\n' {
                    continue;
                }
            }
            match b {
                b'\n' => self.end_line(&mut out),
                b'\r' => {
                    self.after_cr = true;
                    self.end_line(&mut out);
                }
                _ => self.line.push(b),
            }
        }
        out
    }

    /// The body ended: a last frame the server did not end with a blank line is dropped, as the
    /// format says, so a cut-off stream never yields half a frame.
    pub fn finish(&mut self) {
        self.line.clear();
        self.event = None;
        self.data = None;
    }

    fn end_line(&mut self, out: &mut Vec<SseFrame>) {
        let mut raw = std::mem::take(&mut self.line);
        if !self.started {
            self.started = true;
            if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
                raw.drain(..3);
            }
        }
        if raw.is_empty() {
            if let Some(data) = self.data.take() {
                out.push(SseFrame {
                    event: self.event.take(),
                    data,
                });
            }
            self.event = None;
            return;
        }
        let line = String::from_utf8_lossy(&raw);
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => match &mut self.data {
                Some(d) => {
                    d.push('\n');
                    d.push_str(value);
                }
                None => self.data = Some(value.to_owned()),
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = "\u{feff}: a comment\r\n\
        event: message_start\r\n\
        data: {\"type\":\"message_start\"}\r\n\
        \r\n\
        event: content_block_delta\n\
        data: {\"text\":\"h\u{e9}llo \u{1f600}\"}\n\
        \n\
        data:first\n\
        data: second\n\
        id: 7\n\
        retry: 100\n\
        \n\
        event: ping\r\
        data: {}\r\
        \r\
        event: lonely\n\
        \n\
        event: message_stop\n\
        data: {\"type\":\"message_stop\"}\n\
        \n";

    fn expected() -> Vec<SseFrame> {
        let f = |e: Option<&str>, d: &str| SseFrame {
            event: e.map(str::to_owned),
            data: d.to_owned(),
        };
        vec![
            f(Some("message_start"), "{\"type\":\"message_start\"}"),
            f(
                Some("content_block_delta"),
                "{\"text\":\"h\u{e9}llo \u{1f600}\"}",
            ),
            f(None, "first\nsecond"),
            f(Some("ping"), "{}"),
            f(Some("message_stop"), "{\"type\":\"message_stop\"}"),
        ]
    }

    #[test]
    fn whole_stream_at_once() {
        let mut p = SseParser::new();
        assert_eq!(p.feed(STREAM.as_bytes()), expected());
    }

    #[test]
    fn split_at_every_byte_boundary() {
        let bytes = STREAM.as_bytes();
        for at in 0..=bytes.len() {
            let mut p = SseParser::new();
            let mut got = p.feed(&bytes[..at]);
            got.extend(p.feed(&bytes[at..]));
            assert_eq!(got, expected(), "split at byte {at}");
        }
    }

    #[test]
    fn split_at_every_pair_of_boundaries() {
        let bytes = STREAM.as_bytes();
        for a in 0..=bytes.len() {
            for b in a..=bytes.len() {
                let mut p = SseParser::new();
                let mut got = p.feed(&bytes[..a]);
                got.extend(p.feed(&bytes[a..b]));
                got.extend(p.feed(&bytes[b..]));
                assert_eq!(got, expected(), "split at bytes {a} and {b}");
            }
        }
    }

    #[test]
    fn one_byte_at_a_time() {
        let mut p = SseParser::new();
        let got: Vec<_> = STREAM
            .as_bytes()
            .iter()
            .flat_map(|b| p.feed(&[*b]))
            .collect();
        assert_eq!(got, expected());
    }

    #[test]
    fn an_unended_last_frame_is_dropped() {
        let mut p = SseParser::new();
        let got = p.feed(b"event: a\ndata: 1\n\nevent: b\ndata: 2\n");
        p.finish();
        assert_eq!(got.len(), 1);
        assert_eq!(p.feed(b"\n"), Vec::new());
    }
}
