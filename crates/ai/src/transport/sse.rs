//! `_sse_payloads`: Server-Sent Events as OpenAI-compatible servers send
//! them, with Python's reading kept exactly (design §7, item 10):
//!
//! - only `\n` ends a line; trailing `\r`s are stripped, so a bare CR does
//!   not end a line;
//! - `data:` loses all leading spaces, not just one;
//! - multi-line `data` joins with `\n`; an empty line ends the event;
//! - comments (`:`) and other fields are ignored;
//! - a payload that is not a JSON object is skipped; `[DONE]` ends the
//!   stream.

use kassiber_envelope::AppError;
use kassiber_pyjson::Value;

pub const DONE_SENTINEL: &str = "[DONE]";

/// One parsed event.
#[derive(Debug, PartialEq)]
pub enum Event {
    Payload(Value),
    /// The `[DONE]` sentinel; nothing after it is read.
    Done,
}

#[derive(Default)]
pub struct Splitter {
    buffer: Vec<String>,
}

impl Splitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one line (with or without its `\n`).
    pub fn line(&mut self, raw: &str) -> Result<Option<Event>, AppError> {
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');
        if line.is_empty() {
            return self.take();
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        if let Some(data) = line.strip_prefix("data:") {
            self.buffer.push(data.trim_start_matches(' ').to_owned());
        }
        Ok(None)
    }

    /// The end of the stream: the last buffered event, if any.
    pub fn finish(&mut self) -> Result<Option<Event>, AppError> {
        self.take()
    }

    fn take(&mut self) -> Result<Option<Event>, AppError> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        let payload = self.buffer.join("\n");
        self.buffer.clear();
        if payload == DONE_SENTINEL {
            return Ok(Some(Event::Done));
        }
        match kassiber_pytext::loads(&payload) {
            Ok(value @ Value::Dict(_)) => Ok(Some(Event::Payload(value))),
            Ok(_) => Ok(None),
            Err(error) if error.is_decode_error() => Ok(None),
            // Python lets a RecursionError or an oversized integer escape.
            Err(error) => Err(AppError::new("internal_error", error.to_string())),
        }
    }
}

/// `parse_sse_chunks(lines)`: the payloads up to `[DONE]`.
pub fn parse_chunks<'a>(lines: impl IntoIterator<Item = &'a str>) -> Result<Vec<Value>, AppError> {
    let mut splitter = Splitter::new();
    let mut payloads = Vec::new();
    for line in lines {
        match splitter.line(line)? {
            Some(Event::Payload(value)) => payloads.push(value),
            Some(Event::Done) => return Ok(payloads),
            None => {}
        }
    }
    if let Some(Event::Payload(value)) = splitter.finish()? {
        payloads.push(value);
    }
    Ok(payloads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::dict;

    fn obj(n: i64) -> Value {
        dict([("n", Value::from(n))])
    }

    #[test]
    fn splits_events_like_python() {
        let lines = [
            ": OPENROUTER PROCESSING\n",
            "\n",
            "event: message\n",
            "id: 1\n",
            "data: {\"n\":\n",
            "data:    1}\n",
            "\n",
            "data: not json\n",
            "\n",
            "data: [1, 2]\n",
            "\n",
            "data:{\"n\":2}\r\r\n",
            "\r\n",
            "data: {\"n\":3}",
        ];
        assert_eq!(parse_chunks(lines).unwrap(), [obj(1), obj(2), obj(3)]);
    }

    #[test]
    fn done_stops_and_quirks_hold() {
        assert_eq!(
            parse_chunks([
                "data: {\"n\":1}\n",
                "\n",
                "data: [DONE]\n",
                "\n",
                "data: {\"n\":2}\n",
                "\n"
            ])
            .unwrap(),
            [obj(1)]
        );
        // `[DONE]` with a trailing space is a payload (and not JSON).
        assert_eq!(
            parse_chunks(["data: [DONE] \n", "\n", "data: {\"n\":2}\n"]).unwrap(),
            [obj(2)]
        );
        // A bare CR does not end a line: both fields are one data line.
        assert_eq!(
            parse_chunks(["data: {\"n\":1}\rdata: {\"n\":2}\n", "\n"]).unwrap(),
            []
        );
        // `data:` with no space, and two lines joined by a newline.
        assert_eq!(
            parse_chunks(["data:{\"n\":\n", "data:4}\n"]).unwrap(),
            [obj(4)]
        );
        // `data` without a colon is not a data field.
        assert_eq!(parse_chunks(["data {\"n\":1}\n", "\n"]).unwrap(), []);
        let mut splitter = Splitter::new();
        assert_eq!(splitter.line("data: [DONE]").unwrap(), None);
        assert_eq!(splitter.finish().unwrap(), Some(Event::Done));
    }
}
