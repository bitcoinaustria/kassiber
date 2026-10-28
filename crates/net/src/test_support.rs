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
