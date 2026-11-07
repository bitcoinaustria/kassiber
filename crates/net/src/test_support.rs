//! Owned loopback HTTP fake for downstream service tests. Test builds only.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
pub struct FakeProvider {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
pub type HttpReply = (u16, Vec<(String, String)>, String);
impl FakeProvider {
    pub fn new(body: &str) -> std::io::Result<Self> {
        Self::replies(vec![(200, body.to_owned())])
    }
    /// Repeats the last reply for further requests, without external I/O.
    pub fn replies(replies: Vec<(u16, String)>) -> std::io::Result<Self> {
        Self::script(
            replies
                .into_iter()
                .map(|(status, body)| (status, Vec::new(), body))
                .collect(),
        )
    }
    /// Explicit headers permit deterministic Retry-After and redirect fixtures.
    pub fn script(replies: Vec<HttpReply>) -> std::io::Result<Self> {
        assert!(!replies.is_empty());
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let requests = hits.clone();
        let thread = thread::spawn(move || {
            let mut index = 0;
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => break,
                };
                // macOS hands out accepted sockets in the listener's
                // non-blocking mode; Linux does not.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let mut bytes = Vec::new();
                let mut byte = [0; 1];
                while bytes.len() < 65536 {
                    if stream.read_exact(&mut byte).is_err() {
                        break;
                    }
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let header = String::from_utf8_lossy(&bytes);
                let length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0)
                    .min(16 * 1024 * 1024);
                let mut body = vec![0; length];
                if stream.read_exact(&mut body).is_err() {
                    continue;
                }
                bytes.extend(body);
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&bytes).into_owned());
                let (status, headers, body) = &replies[index.min(replies.len() - 1)];
                index += 1;
                let extra: String = headers
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}\r\n"))
                    .collect();
                let _=write!(stream,"HTTP/1.1 {status} Test\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            }
        });
        Ok(Self {
            base,
            hits,
            stop,
            thread: Some(thread),
        })
    }
    pub fn base(&self) -> &str {
        &self.base
    }
    pub fn requests(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}
impl Drop for FakeProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A single owned raw session. Each step reads a fixed number of request lines
/// before writing its fixture bytes, allowing reordered and duplicate RPC IDs.
pub struct FakeElectrum {
    url: String,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl FakeElectrum {
    pub fn script(steps: Vec<(usize, String)>) -> std::io::Result<Self> {
        use std::io::{BufRead, BufReader};
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = format!("tcp://{}", listener.local_addr()?);
        let hits = Arc::new(Mutex::new(Vec::new()));
        let requests = hits.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            let stream = loop {
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => return,
                }
            };
            // macOS hands out accepted sockets in the listener's
            // non-blocking mode; Linux does not.
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream);
            for (count, response) in steps {
                for _ in 0..count {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    requests.lock().unwrap().push(line);
                }
                if reader.get_mut().write_all(response.as_bytes()).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            url,
            hits,
            stop,
            thread: Some(thread),
        })
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn requests(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}
impl Drop for FakeElectrum {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One scripted answer of a [`ScriptedProvider`].
#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    /// Send the body as chunks of this many bytes, with chunked framing.
    pub chunk_size: Option<usize>,
    /// Keep the connection open after the body until the provider drops.
    pub hold: bool,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self::raw(status, "application/json", body.as_bytes())
    }

    /// `data: <event>\n\n` per event, then `data: [DONE]` when `done`.
    pub fn sse(events: &[&str], done: bool) -> Self {
        let mut body = String::new();
        for event in events {
            body.push_str(&format!("data: {event}\n\n"));
        }
        if done {
            body.push_str("data: [DONE]\n\n");
        }
        Self::raw(200, "text/event-stream", body.as_bytes())
    }

    pub fn raw(status: u16, content_type: &str, body: &[u8]) -> Self {
        Self {
            status,
            content_type: content_type.into(),
            body: body.to_vec(),
            chunk_size: None,
            hold: false,
        }
    }

    pub fn chunked(mut self, size: usize) -> Self {
        self.chunk_size = Some(size.max(1));
        self
    }

    pub fn held(mut self) -> Self {
        self.hold = true;
        self
    }
}

/// A request a [`ScriptedProvider`] received.
#[derive(Clone, Debug)]
pub struct Received {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    pub authorization: Option<String>,
    pub accept: Option<String>,
}

#[derive(Default)]
struct Script {
    routes: Vec<((String, String), Vec<Reply>)>,
    requests: Vec<Received>,
}

/// The Rust twin of the Python suites' `ScriptedProvider`: answers each
/// (method, path) from its own queue, 404 `{"detail": "Not Found"}` when
/// nothing is queued, on loopback only. Test builds only.
pub struct ScriptedProvider {
    port: u16,
    script: Arc<Mutex<Script>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ScriptedProvider {
    pub fn new() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let script = Arc::new(Mutex::new(Script::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopped) = (script.clone(), stop.clone());
        let thread = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => break,
                };
                let (shared, stopped) = (shared.clone(), stopped.clone());
                workers.push(thread::spawn(move || serve(stream, &shared, &stopped)));
            }
            for worker in workers {
                let _ = worker.join();
            }
        });
        Ok(Self {
            port,
            script,
            stop,
            thread: Some(thread),
        })
    }

    /// `http://127.0.0.1:<port>/v1`.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Queues replies for `method` and `/v1/<path>`.
    pub fn queue(&self, method: &str, path: &str, replies: Vec<Reply>) {
        let key = (method.to_owned(), format!("/v1/{path}"));
        let mut script = self.script.lock().unwrap();
        match script.routes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, queue)) => queue.extend(replies),
            None => script.routes.push((key, replies)),
        }
    }

    pub fn requests(&self) -> Vec<Received> {
        self.script.lock().unwrap().requests.clone()
    }

    pub fn clear_requests(&self) {
        self.script.lock().unwrap().requests.clear();
    }

    /// The raw bodies sent to `/v1/<path>`.
    pub fn bodies(&self, path: &str) -> Vec<Vec<u8>> {
        let path = format!("/v1/{path}");
        self.requests()
            .into_iter()
            .filter(|r| r.path == path)
            .map(|r| r.body)
            .collect()
    }
}

fn serve(mut stream: std::net::TcpStream, script: &Mutex<Script>, stop: &AtomicBool) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while head.len() < 65536 && !head.ends_with(b"\r\n\r\n") {
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let method = first.next().unwrap_or_default().to_owned();
    let path = first.next().unwrap_or_default().to_owned();
    let header = |name: &str| {
        text.split("\r\n").skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    };
    let length = header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(16 * 1024 * 1024);
    let mut body = vec![0; length];
    if stream.read_exact(&mut body).is_err() {
        return;
    }
    let reply = {
        let mut script = script.lock().unwrap();
        script.requests.push(Received {
            method: method.clone(),
            path: path.clone(),
            body,
            authorization: header("authorization"),
            accept: header("accept"),
        });
        script
            .routes
            .iter_mut()
            .find(|(key, _)| *key == (method.clone(), path.clone()))
            .and_then(|(_, queue)| (!queue.is_empty()).then(|| queue.remove(0)))
    }
    .unwrap_or_else(|| Reply::json(404, r#"{"detail": "Not Found"}"#));
    let framing = match reply.chunk_size {
        Some(_) => "transfer-encoding: chunked".to_owned(),
        None => format!("content-length: {}", reply.body.len()),
    };
    let head = format!(
        "HTTP/1.1 {} Scripted\r\ncontent-type: {}\r\n{framing}\r\nconnection: close\r\n\r\n",
        reply.status, reply.content_type
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    match reply.chunk_size {
        Some(size) => {
            for piece in reply.body.chunks(size) {
                let mut chunk = format!("{:x}\r\n", piece.len()).into_bytes();
                chunk.extend_from_slice(piece);
                chunk.extend_from_slice(b"\r\n");
                if stream.write_all(&chunk).is_err() || stream.flush().is_err() {
                    return;
                }
            }
            if !reply.hold {
                let _ = stream.write_all(b"0\r\n\r\n");
            }
        }
        None => {
            let _ = stream.write_all(&reply.body);
        }
    }
    let _ = stream.flush();
    while reply.hold && !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(5));
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
