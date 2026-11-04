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
    Process,
}

#[derive(Clone, Copy)]
pub(crate) enum WriteContext {
    Raw,
    Http(&'static str),
    ProxyHandshake,
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
    /// The last evicted record id, so callers can compute their own gap.
    pub evicted_through: u64,
}

static PROCESS: std::sync::OnceLock<(Ledger, String)> = std::sync::OnceLock::new();

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
    /// The process-wide ledger every product gate records into, like
    /// Python's `get_egress_ledger()`. Clones share one buffer.
    pub fn process() -> Ledger {
        PROCESS.get_or_init(started).0.clone()
    }
    /// When the process ledger was created, as Python's whole-second
    /// `now_iso()`.
    pub fn process_started_at() -> String {
        PROCESS.get_or_init(started).1.clone()
    }

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
        self.record_with_method(request, endpoint, operation, bytes_out, None);
    }

    fn record_with_method(
        &self,
        request: &Request,
        endpoint: &Endpoint,
        operation: Operation,
        bytes_out: usize,
        method: Option<&'static str>,
    ) {
        let mut buffer = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        buffer.last_id += 1;
        let record = Record {
            id: buffer.last_id,
            ts: SystemTime::now(),
            subsystem: request.subsystem,
            host: endpoint.host.clone(),
            port: endpoint.port,
            scheme: if request.cln.is_some() {
                "cln"
            } else if request.tls == crate::TlsTrust::Plaintext {
                "tcp"
            } else {
                "tls"
            },
            operation,
            method,
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

    pub(crate) fn record_write(&self, request: &Request, context: WriteContext, bytes: usize) {
        let (endpoint, operation, method) = match context {
            WriteContext::Raw => (&request.endpoint, Operation::Write, None),
            WriteContext::Http(method) => (&request.endpoint, Operation::Write, Some(method)),
            WriteContext::ProxyHandshake => (
                request
                    .proxy
                    .as_ref()
                    .map_or(&request.endpoint, |proxy| &proxy.endpoint),
                Operation::ProxyHandshake,
                Some("CONNECT"),
            ),
        };
        self.record_with_method(request, endpoint, operation, bytes, method);
    }

    /// Every retained record, uncapped, for whole-buffer summaries.
    pub fn retained(&self) -> Snapshot {
        let buffer = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            records: buffer.records.iter().map(|(r, _)| r.clone()).collect(),
            last_id: buffer.last_id,
            gap: buffer.evicted_through > 0,
            buffer_bytes: buffer.bytes,
            evicted_through: buffer.evicted_through,
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
            evicted_through: buffer.evicted_through,
        }
    }
}

/// The process ledger and its creation time as Python's `now_iso()`.
fn started() -> (Ledger, String) {
    let seconds = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let stamp = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    );
    (Ledger::default(), stamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Proxy, TlsTrust};

    #[test]
    fn http_and_connect_writes_log_only_closed_route_metadata() {
        let proxy = Proxy::new(
            "http",
            Endpoint::new("proxy.invalid", 8080).unwrap(),
            Some(("private-user".into(), "private-password".into())),
        )
        .unwrap();
        let request = Request::new(
            Subsystem::Sync,
            Endpoint::new("target.invalid", 443).unwrap(),
            TlsTrust::BundledRoots,
        )
        .with_proxy(proxy);
        let ledger = Ledger::default();
        ledger.record_write(&request, WriteContext::ProxyHandshake, 120);
        ledger.record_write(&request, WriteContext::Http("POST"), 250);
        let records = ledger.snapshot(0, 2000).records;
        assert_eq!(records[0].operation, Operation::ProxyHandshake);
        assert_eq!(records[0].host, "proxy.invalid");
        assert_eq!(records[0].method, Some("CONNECT"));
        assert_eq!(records[1].operation, Operation::Write);
        assert_eq!(records[1].host, "target.invalid");
        assert_eq!(records[1].method, Some("POST"));
        let output = format!("{records:?}");
        for secret in ["private-user", "private-password", "Proxy-Authorization"] {
            assert!(!output.contains(secret));
        }
    }
}
