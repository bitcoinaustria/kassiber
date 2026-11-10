use super::*;
use kassiber_net::{Connection, Gate};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};

/// Bitcoin and Liquid use the same newline-delimited Electrum protocol. A batch
/// is multiple request lines, not a JSON array. Responses may be out of order.
pub struct ElectrumClient {
    backend: Backend,
    reader: BufReader<Connection>,
    next_id: u64,
    pub server_version: Value,
}
impl ElectrumClient {
    /// Full-header anchor for compatibility history and timestamp caches.
    /// Height alone cannot distinguish a same-height reorg.
    pub fn tip_fingerprint(&mut self, deadline: Instant) -> Option<String> {
        self.tip_anchor(deadline).0
    }
    /// `_electrum_tip_fingerprint(client, anchor=...)`: the tip header's
    /// fingerprint and the tip height as a cache anchor (a block height a
    /// checkpoint can hold). Either is absent when the reply cannot prove it.
    pub fn tip_anchor(&mut self, deadline: Instant) -> (Option<String>, Option<u32>) {
        let Ok(tip) = self.call("blockchain.headers.subscribe", Some(&json!([])), deadline) else {
            return (None, None);
        };
        let height = match tip.get("height") {
            Some(Value::Number(n)) if n.to_string().bytes().all(|b| b.is_ascii_digit()) => n,
            _ => return (None, None),
        };
        let anchor = n_to_u32(height);
        (tip_fingerprint(&tip), anchor)
    }
    pub fn batch_size(&self) -> usize {
        self.backend.batch_size
    }
    /// Explicit connection includes Python's server.version negotiation.
    pub fn connect(
        backend: Backend,
        gate: &Gate,
        consent: Option<Consent>,
        deadline: Instant,
    ) -> Result<Self> {
        let route = gate
            .authorize(backend.request.clone(), consent)
            .map_err(|e| e.app_error())?;
        let mut connection = route
            .connect(backend.timeout.min(remaining(deadline)?))
            .map_err(|e| e.app_error())?;
        connection.set_deadline(Some(deadline));
        let mut client = Self {
            backend,
            reader: BufReader::new(connection),
            next_id: 0,
            server_version: Value::Null,
        };
        client.server_version = client.call(
            "server.version",
            Some(&json!(["Kassiber", "1.6"])),
            deadline,
        )?;
        Ok(client)
    }
    pub fn call(
        &mut self,
        method: &str,
        params: Option<&Value>,
        deadline: Instant,
    ) -> Result<Value> {
        let mut results = self.batch_call(
            &[(method.to_owned(), params.cloned().unwrap_or(json!([])))],
            deadline,
        )?;
        Ok(results.remove(0))
    }
    pub fn batch_call(
        &mut self,
        requests: &[(String, Value)],
        deadline: Instant,
    ) -> Result<Vec<Value>> {
        // Validate the whole action before the first write, including later chunks.
        for (method, _) in requests {
            validate_method(method)?;
        }
        let mut results = Vec::with_capacity(requests.len());
        for chunk in requests.chunks(self.backend.batch_size) {
            results.extend(self.batch_chunk(chunk, deadline)?);
        }
        Ok(results)
    }
    fn batch_chunk(
        &mut self,
        requests: &[(String, Value)],
        deadline: Instant,
    ) -> Result<Vec<Value>> {
        remaining(deadline)?;
        self.reader.get_mut().set_deadline(Some(deadline));
        let mut pending = BTreeMap::new();
        let mut payload = Vec::new();
        for (index, (method, params)) in requests.iter().enumerate() {
            self.next_id = self
                .next_id
                .checked_add(1)
                .ok_or_else(|| app("Electrum request IDs exhausted"))?;
            pending.insert(self.next_id, (index, method));
            let params = if truthy(params) {
                params.clone()
            } else {
                json!([])
            };
            let request =
                json!({"jsonrpc":"2.0", "id":self.next_id, "method":method, "params":params});
            serde_json::to_writer(&mut payload, &request).expect("JSON value");
            payload.push(b'\n');
        }
        if payload.len() > RESPONSE_LIMIT {
            return Err(AppError::new(
                "protocol_error",
                "Electrum request exceeds size limit",
            ));
        }
        self.reader
            .get_mut()
            .write_all(&payload)
            .map_err(io_error)?;
        let mut results = vec![Value::Null; requests.len()];
        let mut consumed = 0;
        while !pending.is_empty() {
            remaining(deadline)?;
            let mut line = Vec::new();
            // Bounded even when the peer never sends a newline. Total bytes,
            // including notifications/duplicate IDs, share the request budget.
            let count = self
                .reader
                .by_ref()
                .take((RESPONSE_LIMIT - consumed + 1) as u64)
                .read_until(b'\n', &mut line)
                .map_err(io_error)?;
            consumed += count;
            if consumed > RESPONSE_LIMIT {
                return Err(AppError::new(
                    "protocol_error",
                    "Electrum response exceeds size limit",
                ));
            }
            if count == 0 {
                return Err(app(format!(
                    "Electrum backend '{}' closed the connection",
                    self.backend.name
                )));
            }
            let message =
                serde_json::from_slice::<Value>(&line).map_err(|_| self.format_error())?;
            if !message.is_object() {
                return Err(self.format_error());
            }
            let Some(id) = message.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let Some((index, method)) = pending.remove(&id) else {
                continue;
            };
            if let Some(error) = message.get("error").filter(|e| truthy(e)) {
                let detail = if let Some(obj) = error.as_object() {
                    format!(
                        "({}): {}",
                        obj.get("code")
                            .map(kassiber_value)
                            .unwrap_or_else(|| "unknown".into()),
                        obj.get("message")
                            .map(kassiber_value)
                            .unwrap_or_else(|| kassiber_value(error))
                    )
                } else {
                    kassiber_value(error)
                };
                // The server's text may repeat a backend credential.
                let detail =
                    kassiber_credentials::withhold(&detail, &self.backend.credential_forms(&[]));
                return Err(AppError::new(
                    "electrum_rpc_error",
                    format!("Electrum call {method} failed {}", redacted(&detail)),
                ));
            }
            results[index] = message.get("result").cloned().unwrap_or(Value::Null);
        }
        Ok(results)
    }
    fn format_error(&self) -> AppError {
        app(format!("Backend '{}' did not respond with Electrum-format JSON",self.backend.name))
            .with_hint("Check that the backend URL points to an Electrum server and uses the correct tcp/ssl port.").retryable(true)
    }
}
fn kassiber_value(value: &Value) -> String {
    kassiber_envelope::py_str(&details(value.clone()))
}
fn validate_method(method: &str) -> Result<()> {
    if [
        "server.version",
        "server.banner",
        "server.features",
        "blockchain.headers.subscribe",
        "blockchain.block.header",
        "blockchain.block.headers",
        "blockchain.scripthash.get_history",
        "blockchain.scripthash.get_balance",
        "blockchain.scripthash.listunspent",
        "blockchain.scripthash.subscribe",
        "blockchain.transaction.get",
        "blockchain.transaction.get_merkle",
        "blockchain.estimatefee",
        "mempool.get_fee_histogram",
    ]
    .contains(&method)
    {
        Ok(())
    } else {
        Err(AppError::new(
            "validation",
            "Unsupported watch-only Electrum method",
        ))
    }
}

fn n_to_u32(n: &serde_json::Number) -> Option<u32> {
    n.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn tip_fingerprint(tip: &Value) -> Option<String> {
    match tip.get("height")? {
        Value::Number(n) if n.to_string().bytes().all(|b| b.is_ascii_digit()) => {}
        _ => return None,
    }
    header_fingerprint(tip.get("hex")?)
}

/// `_electrum_header_fingerprint`: SHA-256 of a hex header of at least 80
/// bytes. Cache anchors bind to the whole header, not its height.
pub fn header_fingerprint(header: &Value) -> Option<String> {
    use sha2::{Digest, Sha256};
    let text = header.as_str()?;
    // CPython bytes.fromhex permits ASCII whitespace between whole bytes.
    let mut raw = Vec::new();
    for pair in text.split([' ', '\t', '\n', '\r', '\x0b', '\x0c']) {
        if !pair.len().is_multiple_of(2) {
            return None;
        }
        for digits in pair.as_bytes().as_chunks::<2>().0 {
            let digits = std::str::from_utf8(digits).ok()?;
            raw.push(u8::from_str_radix(digits, 16).ok()?);
        }
    }
    (raw.len() >= 80).then(|| {
        Sha256::digest(raw)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    })
}

#[cfg(test)]
mod tip_tests {
    use super::*;
    #[test]
    fn compatibility_tip_fingerprint_binds_full_header_and_rejects_unknown_anchors() {
        let header = "00".repeat(80);
        let expected = "5b6fb58e61fa475939767d68a446f97f1bff02c0e5935a3ea8bb51e6515783d8";
        assert_eq!(
            tip_fingerprint(&json!({"height":5,"hex":header})).as_deref(),
            Some(expected)
        );
        assert_eq!(
            tip_fingerprint(&json!({"height":6,"hex":"00 ".repeat(80)})).as_deref(),
            Some(expected)
        );
        for changed in [format!("01{}", "00".repeat(79)), "00".repeat(81)] {
            assert_ne!(
                tip_fingerprint(&json!({"height":5,"hex":changed})).as_deref(),
                Some(expected)
            );
        }
        for unknown in [
            json!(null),
            json!({}),
            json!({"height":true,"hex":header}),
            json!({"height":5.0,"hex":header}),
            json!({"height":-1,"hex":header}),
            json!({"height":"5","hex":header}),
            json!({"height":5,"hex":"00".repeat(79)}),
            json!({"height":5,"hex":"0 0".repeat(80)}),
            json!({"height":5,"hex":"gg".repeat(80)}),
            json!({"height":5,"hex":false}),
        ] {
            assert_eq!(tip_fingerprint(&unknown), None, "{unknown}");
        }
    }
}
