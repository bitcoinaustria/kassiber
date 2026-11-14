//! HTTP to the OpenCode server the adapter itself started, and only to it.
//!
//! Every request goes through `kassiber-net`, the one crate that may open a
//! socket: a route to `127.0.0.1:<port>` classified as an on-device AI
//! endpoint (loopback, no proxy), with an explicit-trigger consent (the
//! daemon starts a broker only for a turn or probe the user asked for, after
//! its own online gate). The client follows no redirects, sends no ambient
//! credentials and reads no proxy settings. The provider's own outbound
//! traffic stays the provider's, as with every other CLI provider.

use std::time::{Duration, Instant};

use kassiber_net::{
    Cancel, Consent, Endpoint, Gate, HttpOptions, HttpRequest, HttpStream, Ledger, Offline,
    RedirectPolicy, Request, Subsystem, TlsTrust,
};

/// The loopback server of one OpenCode process.
#[derive(Clone)]
pub struct Loopback {
    gate: Gate,
    port: u16,
}

/// What a request needs beyond its method and target.
#[derive(Default)]
pub struct Call<'a> {
    pub headers: Vec<(&'a str, String)>,
    pub body: Option<Vec<u8>>,
    /// Bounds each blocking read and write.
    pub timeout: Option<Duration>,
    /// Bounds the whole request, body included.
    pub deadline: Option<Instant>,
    pub cancel: Option<&'a Cancel>,
    pub max_response_bytes: Option<usize>,
}

impl Loopback {
    pub fn new(port: u16) -> Self {
        // The broker process has no offline preference of its own: the daemon
        // checked it before starting this broker, and an on-device route is
        // allowed in offline mode anyway.
        Self {
            gate: Gate::new(|| Offline::Online, Ledger::default()),
            port,
        }
    }

    /// `http://127.0.0.1:<port>`.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Send `method` to `target` (a path with its query) and return once the
    /// response head has arrived.
    pub fn open(
        &self,
        method: &str,
        target: &str,
        call: Call<'_>,
    ) -> Result<HttpStream, kassiber_net::Error> {
        let endpoint = Endpoint::new("127.0.0.1", self.port)?;
        let request = Request::new(Subsystem::Ai, endpoint, TlsTrust::Plaintext).on_device_ai();
        let consent = Consent::explicit_trigger(&request);
        let route = self.gate.authorize(request, Some(consent))?;
        // `fetch`'s default, which the TypeScript broker sent.
        let mut http = HttpRequest::new(method, &format!("{}{target}", self.origin()))?
            .header("Accept", "*/*")?;
        for (name, value) in &call.headers {
            http = http.header(name, value)?;
        }
        if let Some(body) = call.body {
            http = http.body(body);
        }
        if let Some(cancel) = call.cancel {
            http = http.cancellable(cancel);
        }
        let defaults = HttpOptions::default();
        http.open(
            route,
            HttpOptions {
                timeout: call.timeout.unwrap_or(defaults.timeout),
                deadline: call.deadline,
                max_response_bytes: call
                    .max_response_bytes
                    .unwrap_or(defaults.max_response_bytes),
                redirects: RedirectPolicy::Refuse,
            },
        )
    }
}

/// `application/x-www-form-urlencoded`, as `URLSearchParams` serializes a
/// query value.
pub fn form_encode(text: &str) -> String {
    let mut url = kassiber_net::HttpUrl::parse("http://127.0.0.1/").expect("a fixed URL");
    url.query_pairs_mut().append_pair("v", text);
    url.query().unwrap_or("v=")[2..].to_owned()
}

/// Why [`Frames::next`] stopped.
pub enum FrameError {
    Read(kassiber_net::Error),
    TooLarge,
}

/// Server-sent event frames, read off a response body as they arrive.
pub struct Frames {
    stream: HttpStream,
    buffer: Vec<u8>,
}

impl Frames {
    pub fn new(stream: HttpStream) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
        }
    }

    /// The next frame's text, before its blank line, with `\r\n` already
    /// turned into `\n`; `Ok(None)` when the body ended. `normalize_cr` also
    /// turns a lone `\r` into `\n`, as the OpenCode 1 SDK does. `limit`
    /// bounds the unframed text (in UTF-16 units, as a JavaScript string
    /// buffer counts) after each read.
    pub fn next(
        &mut self,
        normalize_cr: bool,
        limit: Option<usize>,
    ) -> Result<Option<String>, FrameError> {
        loop {
            if let Some(end) = find_blank_line(&self.buffer) {
                let frame: Vec<u8> = self.buffer.drain(..end + 2).take(end).collect();
                return Ok(Some(String::from_utf8_lossy(&frame).into_owned()));
            }
            let mut chunk = [0u8; 8192];
            let read = self
                .stream
                .read_body(&mut chunk)
                .map_err(FrameError::Read)?;
            if read == 0 {
                return Ok(None);
            }
            self.buffer.extend_from_slice(&chunk[..read]);
            if let Some(limit) = limit {
                if self.buffer.len() > limit
                    && super::js::utf16_len(&String::from_utf8_lossy(&self.buffer)) > limit
                {
                    return Err(FrameError::TooLarge);
                }
            }
            normalize(&mut self.buffer, normalize_cr);
        }
    }
}

/// `\r\n` to `\n` (and a lone `\r` to `\n` when asked). A `\r` at the very
/// end waits for the next chunk, as a JavaScript string buffer would see
/// the pair once both halves arrived.
fn normalize(buffer: &mut Vec<u8>, normalize_cr: bool) {
    let mut out = Vec::with_capacity(buffer.len());
    let mut index = 0;
    while index < buffer.len() {
        let byte = buffer[index];
        if byte == b'\r' {
            match buffer.get(index + 1) {
                Some(b'\n') => {
                    out.push(b'\n');
                    index += 2;
                    continue;
                }
                None => {
                    out.push(if normalize_cr { b'\n' } else { b'\r' });
                    index += 1;
                    continue;
                }
                Some(_) if normalize_cr => {
                    out.push(b'\n');
                    index += 1;
                    continue;
                }
                Some(_) => {}
            }
        }
        out.push(byte);
        index += 1;
    }
    *buffer = out;
}

fn find_blank_line(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|pair| pair == b"\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encodes_like_url_search_params() {
        assert_eq!(
            form_encode("/tmp/a b+c&d=é~"),
            "%2Ftmp%2Fa+b%2Bc%26d%3D%C3%A9%7E"
        );
    }

    #[test]
    fn normalizes_line_endings_across_chunks() {
        let mut buffer = b"a\r\nb\rc\r".to_vec();
        normalize(&mut buffer, false);
        assert_eq!(buffer, b"a\nb\rc\r");
        buffer.extend_from_slice(b"\nd");
        normalize(&mut buffer, false);
        assert_eq!(buffer, b"a\nb\rc\nd");
        let mut buffer = b"a\rb".to_vec();
        normalize(&mut buffer, true);
        assert_eq!(buffer, b"a\nb");
    }
}
