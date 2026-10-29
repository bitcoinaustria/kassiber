//! The daemon's stdin: `readline(MAX_REQUEST_LINE_CHARS + 1)` on a thread.
//!
//! Python reads stdin in text mode, so a chunk ends at a newline or after
//! 1,000,001 characters, whichever comes first, and the line cap counts
//! characters, not bytes. CPython opens stdin with `newline="\n"` outside
//! Windows, so there a `\r` is an ordinary character; on Windows it uses
//! universal newlines, where `\r\n` and a lone `\r` both become `\n`. A reader thread queues chunks so the
//! control channel can wait for a reply while ordinary requests queue up
//! behind it.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

/// Whether stdin translates `\r\n` and `\r` to `\n`, as CPython's does on
/// Windows only.
const UNIVERSAL_NEWLINES: bool = cfg!(windows);

/// Longest request line, in characters (`daemon.py` `MAX_REQUEST_LINE_CHARS`).
pub const MAX_REQUEST_LINE_CHARS: usize = 1_000_000;

/// One `readline` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chunk {
    /// Text up to and including `\n`, or `MAX_REQUEST_LINE_CHARS + 1`
    /// characters without one, or the last characters before end of input.
    Text(String),
    /// The line held bytes that are not UTF-8. Python's reader thread dies
    /// on them and the daemon stops answering; the Rust daemon reports the
    /// line instead (bytes up to the next newline are dropped).
    Undecodable,
    /// End of input. Once seen, every later read returns it.
    Eof,
}

impl Chunk {
    /// Whether this chunk ends a line (Python's `line.endswith("\n")`, or
    /// the empty string at end of input).
    pub fn ends_line(&self) -> bool {
        match self {
            Chunk::Text(text) => text.ends_with('\n'),
            Chunk::Undecodable | Chunk::Eof => true,
        }
    }
}

/// Reads chunks from `input` on a thread until end of input or a read error.
pub fn spawn_reader(input: Box<dyn Read + Send>) -> Receiver<Chunk> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("kassiber-daemon-stdin".to_owned())
        .spawn(move || {
            let mut reader = LineReader::new(input);
            loop {
                let chunk = reader.next_chunk();
                let done = chunk == Chunk::Eof;
                if sender.send(chunk).is_err() || done {
                    return;
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|_| {
            // Without a reader the daemon sees end of input and stops.
        });
    receiver
}

/// Decodes UTF-8 and translates newlines, one chunk at a time.
struct LineReader {
    reader: BufReader<Box<dyn Read + Send>>,
    eof: bool,
}

impl LineReader {
    fn new(input: Box<dyn Read + Send>) -> Self {
        LineReader {
            reader: BufReader::new(input),
            eof: false,
        }
    }

    fn peek_byte(&mut self) -> Option<u8> {
        if self.eof {
            return None;
        }
        loop {
            match self.reader.fill_buf() {
                Ok([]) => break,
                Ok(buffer) => return Some(buffer[0]),
                // Python retries an interrupted read.
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        self.eof = true;
        None
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.reader.consume(1);
        Some(byte)
    }

    /// The next character: `Ok(None)` at end of input, `Err(())` for bytes
    /// that are not UTF-8.
    fn next_char(&mut self) -> Result<Option<char>, ()> {
        let Some(lead) = self.next_byte() else {
            return Ok(None);
        };
        let width = match lead {
            0x00..=0x7F => return Ok(Some(char::from(lead))),
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => return Err(()),
        };
        let mut bytes = vec![lead];
        for _ in 1..width {
            match self.peek_byte() {
                Some(byte) if byte & 0xC0 == 0x80 => {
                    bytes.push(byte);
                    self.reader.consume(1);
                }
                _ => return Err(()),
            }
        }
        std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| text.chars().next())
            .map(Some)
            .ok_or(())
    }

    fn skip_to_newline(&mut self) {
        while let Some(byte) = self.next_byte() {
            if byte == b'\n' {
                return;
            }
            if UNIVERSAL_NEWLINES && byte == b'\r' {
                if self.peek_byte() == Some(b'\n') {
                    self.reader.consume(1);
                }
                return;
            }
        }
    }

    fn next_chunk(&mut self) -> Chunk {
        let mut text = String::new();
        let mut count = 0usize;
        while count <= MAX_REQUEST_LINE_CHARS {
            match self.next_char() {
                Ok(None) => break,
                Err(()) => {
                    self.skip_to_newline();
                    return Chunk::Undecodable;
                }
                Ok(Some('\r')) if UNIVERSAL_NEWLINES => {
                    if self.peek_byte() == Some(b'\n') {
                        self.reader.consume(1);
                    }
                    text.push('\n');
                    return Chunk::Text(text);
                }
                Ok(Some('\n')) => {
                    text.push('\n');
                    return Chunk::Text(text);
                }
                Ok(Some(c)) => {
                    text.push(c);
                    count += 1;
                }
            }
        }
        if text.is_empty() {
            Chunk::Eof
        } else {
            Chunk::Text(text)
        }
    }
}

/// Chunks from the reader thread, behind the lines a control-channel wait
/// set aside.
pub struct Input {
    receiver: Receiver<Chunk>,
    deferred: VecDeque<Chunk>,
    eof: bool,
}

impl Input {
    pub fn new(receiver: Receiver<Chunk>) -> Self {
        Input {
            receiver,
            deferred: VecDeque::new(),
            eof: false,
        }
    }

    /// `_next_input_line(ctx)`: a set-aside chunk first, then the next one
    /// read.
    pub fn next_chunk(&mut self) -> Chunk {
        if let Some(chunk) = self.deferred.pop_front() {
            return chunk;
        }
        self.next_new(None).unwrap_or(Chunk::Eof)
    }

    pub fn next_chunk_timeout(&mut self, timeout: Option<Duration>) -> Option<Chunk> {
        self.deferred
            .pop_front()
            .map(Some)
            .unwrap_or_else(|| self.next_new(timeout))
    }

    /// The next chunk read from stdin, ignoring set-aside ones; `None` when
    /// `timeout` passes first.
    pub fn next_new(&mut self, timeout: Option<Duration>) -> Option<Chunk> {
        if self.eof {
            return Some(Chunk::Eof);
        }
        let chunk = match timeout {
            None => self.receiver.recv().unwrap_or(Chunk::Eof),
            Some(timeout) => match self.receiver.recv_timeout(timeout) {
                Ok(chunk) => chunk,
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => Chunk::Eof,
            },
        };
        if chunk == Chunk::Eof {
            self.eof = true;
        }
        Some(chunk)
    }

    /// Sets a chunk aside for the main loop, in arrival order.
    pub fn defer(&mut self, chunk: Chunk) {
        self.deferred.push_back(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(bytes: &[u8]) -> Vec<Chunk> {
        let mut reader = LineReader::new(Box::new(std::io::Cursor::new(bytes.to_vec())));
        let mut out = Vec::new();
        loop {
            let chunk = reader.next_chunk();
            let done = chunk == Chunk::Eof;
            out.push(chunk);
            if done {
                return out;
            }
        }
    }

    fn text(s: &str) -> Chunk {
        Chunk::Text(s.to_owned())
    }

    #[test]
    fn newlines_and_eof() {
        let expected = if UNIVERSAL_NEWLINES {
            vec![text("a\n"), text("b\n"), text("c\n"), text("d"), Chunk::Eof]
        } else {
            vec![text("a\r\n"), text("b\rc\n"), text("d"), Chunk::Eof]
        };
        assert_eq!(chunks(b"a\r\nb\rc\nd"), expected);
        assert_eq!(chunks(b""), vec![Chunk::Eof]);
        assert_eq!(
            chunks(b"\xff\xfe x\nok\n"),
            vec![Chunk::Undecodable, text("ok\n"), Chunk::Eof]
        );
        assert_eq!(chunks("é\n".as_bytes()), vec![text("é\n"), Chunk::Eof]);
    }

    #[test]
    fn chunks_stop_after_the_character_cap() {
        let mut line = "é".repeat(MAX_REQUEST_LINE_CHARS + 5);
        line.push('\n');
        let got = chunks(line.as_bytes());
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], Chunk::Text("é".repeat(MAX_REQUEST_LINE_CHARS + 1)));
        assert_eq!(got[1], text("éééé\n"));
        // MAX characters plus the newline is MAX + 1: one chunk.
        let mut exact = "x".repeat(MAX_REQUEST_LINE_CHARS);
        exact.push('\n');
        assert_eq!(chunks(exact.as_bytes())[0], Chunk::Text(exact.clone()));
    }
}
