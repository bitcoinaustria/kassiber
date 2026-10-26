use crate::{Endpoint, Request};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub const MAX_RECORDS: usize = 5_000;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// Closed labels prevent paths, prompts, credentials or RPC arguments entering RAM logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subsystem {
    Sync,
    Pricing,
    Ai,
    Update,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Dns,
    Connect,
    ProxyHandshake,
    Write,
}

/// The Python ledger's metadata fields. `ts` is UTC system time; wire formatting
/// belongs to the future projection adapter. No payload or URL is accepted here.
#[derive(Clone, Debug)]
pub struct Record {
    pub id: u64,
    pub ts: SystemTime,
    pub subsystem: Subsystem,
    pub host: String,
    pub port: u16,
    pub scheme: &'static str,
    pub operation: Operation,
    pub method: Option<&'static str>,
    pub bytes_out: u64,
    pub via_proxy: bool,
}

#[derive(Debug)]
pub struct Snapshot {
    pub records: Vec<Record>,
    pub last_id: u64,
    pub gap: bool,
    pub buffer_bytes: usize,
}

#[derive(Debug)]
struct Buffer {
    records: VecDeque<(Record, usize)>,
    bytes: usize,
    last_id: u64,
    evicted_through: u64,
}

/// Per-process, bounded RAM metadata. Clones share the buffer; nothing is persisted.
#[derive(Clone, Debug)]
pub struct Ledger {
    inner: Arc<Mutex<Buffer>>,
    max_records: usize,
    max_bytes: usize,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::with_limits(MAX_RECORDS, MAX_BYTES)
    }
}

impl Ledger {
    /// Custom limits may tighten, never enlarge, the product defaults.
    pub fn with_limits(max_records: usize, max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Buffer {
                records: VecDeque::new(),
                bytes: 0,
                last_id: 0,
                evicted_through: 0,
            })),
            max_records: max_records.min(MAX_RECORDS),
            max_bytes: max_bytes.min(MAX_BYTES),
        }
    }

    pub(crate) fn record(
        &self,
        request: &Request,
        endpoint: &Endpoint,
        operation: Operation,
        bytes_out: usize,
    ) {
        let mut buffer = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        buffer.last_id += 1;
        let record = Record {
            id: buffer.last_id,
            ts: SystemTime::now(),
            subsystem: request.subsystem,
            host: endpoint.host.clone(),
            port: endpoint.port,
            scheme: if request.tls == crate::TlsTrust::Plaintext {
                "tcp"
            } else {
                "tls"
            },
            operation,
            method: None,
            bytes_out: bytes_out as u64,
            via_proxy: request.proxy.is_some(),
        };
        // Conservative retained-entry accounting (including queue tuple storage).
        // Unlike Python, even a single oversized entry is evicted.
        let size = std::mem::size_of::<(Record, usize)>() + record.host.capacity();
        buffer.bytes += size;
        buffer.records.push_back((record, size));
        while buffer.records.len() > self.max_records || buffer.bytes > self.max_bytes {
            if let Some((evicted, size)) = buffer.records.pop_front() {
                buffer.bytes -= size;
                buffer.evicted_through = evicted.id;
            }
        }
    }

    /// Cursors expose eviction gaps. Snapshot copies are capped like Python (2,000 rows).
    pub fn snapshot(&self, after_id: u64, limit: usize) -> Snapshot {
        let buffer = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            records: buffer
                .records
                .iter()
                .filter(|(record, _)| record.id > after_id)
                .take(limit.min(2_000))
                .map(|(record, _)| record.clone())
                .collect(),
            last_id: buffer.last_id,
            gap: after_id < buffer.evicted_through,
            buffer_bytes: buffer.bytes,
        }
    }
}
