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
impl FakeProvider {
    pub fn new(body: &str) -> std::io::Result<Self> {
        Self::replies(vec![(200, body.to_owned())])
    }
    /// Repeats the last reply for further requests, without external I/O.
    pub fn replies(replies: Vec<(u16, String)>) -> std::io::Result<Self> {
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
                requests
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&bytes).into_owned());
                let (status, body) = &replies[index.min(replies.len() - 1)];
                index += 1;
                let _=write!(stream,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
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
