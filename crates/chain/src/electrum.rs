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
