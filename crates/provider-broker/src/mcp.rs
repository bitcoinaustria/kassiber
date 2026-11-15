//! The MCP child a provider CLI starts for Kassiber's tools
//! (`runMcpServer` in `native-tools.ts`): `kassiber --provider-broker-mcp
//! <socket> <manifest>`.
//!
//! It serves the manifest's tools over stdio through `kassiber_mcp`'s
//! server and forwards every call over the bridge socket; the broker decides
//! whether the name was advertised, and the daemon runs (and validates) the
//! call. Argument schemas are advertised exactly as the daemon sent them.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use kassiber_envelope::{dict, get, s};
use kassiber_mcp::protocol::{Cancelled, Internal, McpServer, ToolOutcome, ToolProvider};
use kassiber_pyjson::{Options, PyStr, Value};

use crate::bridge::{connect, read_line_capped};
use crate::protocol::{safe_error_message, truthy, BrokerFailure};
use crate::workdir::random_uuid;

/// The server's self-description.
pub const SERVER_NAME: &str = "kassiber";
pub const SERVER_VERSION: &str = "1.0.0";
pub const INSTRUCTIONS: &str = "Typed, capability-scoped Kassiber accounting tools.";

/// The manifest's tools, forwarded over the bridge.
pub struct BridgeTools {
    socket_path: String,
    /// The advertised definitions, in manifest order.
    tools: Vec<Value>,
    names: HashSet<String>,
}

impl BridgeTools {
    /// Read the manifest the broker wrote.
    pub fn load(socket_path: &str, manifest: &Path) -> Result<Self, BrokerFailure> {
        let invalid = || BrokerFailure::new("Invalid Kassiber MCP tool manifest.");
        let text = std::fs::read_to_string(manifest)
            .map_err(|error| crate::executables::fs_failure("open", manifest, &error))?;
        let Ok(Value::List(entries)) = kassiber_pytext::loads(&text) else {
            return Err(invalid());
        };
        let mut tools = Vec::new();
        let mut names = HashSet::new();
        for entry in &entries {
            let Some(Value::Str(name)) = get(entry, "name") else {
                return Err(invalid());
            };
            let name = name.as_str().ok_or_else(invalid)?.to_owned();
            let parameters = get(entry, "parameters").cloned().ok_or_else(invalid)?;
            if !names.insert(name.clone()) {
                return Err(BrokerFailure::new(format!(
                    "Tool {name} is already registered"
                )));
            }
            let flag = |key| Value::Bool(matches!(get(entry, key), Some(Value::Bool(true))));
            let mut fields = vec![("name", s(name.as_str()))];
            if let Some(description @ Value::Str(_)) = get(entry, "description") {
                fields.push(("description", description.clone()));
            }
            fields.push(("inputSchema", parameters));
            fields.push((
                "annotations",
                dict([
                    ("readOnlyHint", flag("read_only")),
                    ("destructiveHint", flag("destructive")),
                ]),
            ));
            tools.push(dict(fields));
        }
        Ok(Self {
            socket_path: socket_path.to_owned(),
            tools,
            names,
        })
    }

    /// `forwardToolCall(socketPath, name, args)`: the daemon's output, or
    /// the error text the tool result carries.
    pub fn forward(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let options = Options {
            ensure_ascii: false,
            allow_nan: false,
            separators: Some((",".into(), ":".into())),
            ..Options::default()
        };
        let arguments = kassiber_pyjson::dumps(arguments, &options)
            .map_err(|_| "Kassiber could not encode the tool arguments.".to_owned())?;
        let call_id = random_uuid().map_err(|failure| failure.message().to_owned())?;
        let line = format!(
            "{{\"call_id\":{},\"name\":{},\"arguments\":{arguments}}}\n",
            serde_json::Value::String(call_id),
            serde_json::Value::String(name.to_owned()),
        );
        let mut stream = connect(&self.socket_path).map_err(|error| {
            format!(
                "connect {} {}",
                crate::executables::node_code(&error),
                self.socket_path
            )
        })?;
        stream
            .write_all(line.as_bytes())
            .and_then(|()| stream.flush())
            .map_err(|_| "Kassiber tool bridge closed early.".to_owned())?;
        let reply = read_line_capped(&mut stream)
            .ok_or_else(|| "Kassiber tool bridge closed early.".to_owned())?;
        let reply: serde_json::Value = serde_json::from_slice(&reply)
            .map_err(|_| "Kassiber returned an invalid tool result.".to_owned())?;
        match reply.get("output") {
            Some(serde_json::Value::String(output)) => Ok(output.clone()),
            _ => Err(match reply.get("error") {
                Some(error) if truthy(error) => js_string(error),
                _ => "Kassiber tool execution failed.".to_owned(),
            }),
        }
    }
}

/// JavaScript's `String(value)` for a JSON value.
fn js_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| match item {
                serde_json::Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        serde_json::Value::Object(_) => "[object Object]".to_owned(),
        other => other.to_string(),
    }
}

impl ToolProvider for BridgeTools {
    fn instructions(&self) -> String {
        INSTRUCTIONS.to_owned()
    }

    fn tool_definitions(&self) -> Vec<Value> {
        self.tools.clone()
    }

    fn has_tool(&self, name: &PyStr) -> bool {
        name.as_str().is_some_and(|name| self.names.contains(name))
    }

    /// A failed call is a tool error carrying the message, as the MCP SDK
    /// reports a handler that throws.
    fn call_tool(
        &self,
        name: &PyStr,
        arguments: &Value,
        _cancelled: &Cancelled,
        _client_name: Option<&Value>,
    ) -> Result<ToolOutcome, Internal> {
        let name = name.as_str().ok_or(Internal)?;
        let (text, is_error) = match self.forward(name, arguments) {
            Ok(output) => (output, false),
            Err(message) => (message, true),
        };
        Ok(ToolOutcome {
            // No structured content: the tools declare no output schema.
            structured: Value::Null,
            text: PyStr::from(text),
            is_error,
        })
    }
}

/// `runMcpServer([socket, manifest])` on this process's stdio. A failure to
/// start is reported on stderr, redacted, with exit code 1.
pub fn serve_mcp(socket_path: &str, manifest: &str) -> i32 {
    let tools = match BridgeTools::load(socket_path, Path::new(manifest)) {
        Ok(tools) => tools,
        Err(failure) => {
            eprintln!("{}", safe_error_message(failure.message()));
            return 1;
        }
    };
    let server = Arc::new(McpServer::new(Arc::new(tools), SERVER_NAME, SERVER_VERSION));
    kassiber_mcp::protocol::serve(
        server,
        Box::new(std::io::stdin()),
        Box::new(std::io::stdout()),
    );
    0
}

#[cfg(test)]
mod tests;
