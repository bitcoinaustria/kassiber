//! The native tool bridge (`native-tools.ts`).
//!
//! A provider CLI reaches Kassiber's tools through an MCP server it starts
//! itself: `kassiber --provider-broker-mcp <socket> <manifest>`
//! ([`crate::mcp`]). That child forwards each call over a private local
//! channel to the broker, which writes a `tool_call` event and waits for the
//! daemon's `tool_results`.
//!
//! A loopback TCP port is reachable by every local process, so it would need
//! a shared secret, and the only place to hand one to the child is argv,
//! which is world-readable on Linux. The channel is an `AF_UNIX` socket in a
//! 0700 directory instead, so the operating system enforces the boundary and
//! there is no secret to leak. Windows gets a named pipe with an unguessable
//! name whose DACL admits only this user.
//!
//! Calls outside the advertised catalog and repeated pending call ids are
//! refused here; argument schemas are the daemon's to enforce.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::Value;

use crate::cleanup::{track_directory, TrackedDirectory};
use crate::executables::fs_failure;
use crate::protocol::{minify_json, BrokerFailure, BrokerToolDefinition, Event, EventSink};
use crate::workdir::{make_temp_dir, random_uuid};

/// The largest request or reply line either end accepts.
pub const MAX_BRIDGE_MESSAGE_BYTES: usize = 2_000_000;
/// The manifest the MCP child reads, in the turn's working directory.
pub const MANIFEST_NAME: &str = "kassiber-mcp-tools.json";

const OUTSIDE_CATALOG: &str = "Provider requested a tool outside the advertised catalog.";
const REPEATED_CALL: &str = "Provider repeated a pending tool call id.";
const CLOSED: &str = "Kassiber closed the tool bridge.";

/// A pending call's answer: the daemon's output, or why it was refused.
type Reply = Result<Value, String>;

struct Shared {
    allowed: HashSet<String>,
    pending: Mutex<HashMap<String, mpsc::Sender<Reply>>>,
    closed: AtomicBool,
    events: EventSink,
}

impl Shared {
    fn pending(&self) -> MutexGuard<'_, HashMap<String, mpsc::Sender<Reply>>> {
        self.pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn request(
        &self,
        name: &str,
        arguments: &RawValue,
        call_id: Option<&str>,
    ) -> Result<Value, String> {
        if !self.allowed.contains(name) {
            return Err(OUTSIDE_CATALOG.to_owned());
        }
        let call_id = match call_id {
            Some(call_id) => call_id.to_owned(),
            None => random_uuid().map_err(|failure| failure.message().to_owned())?,
        };
        let (sender, receiver) = mpsc::channel();
        {
            let mut pending = self.pending();
            if self.closed.load(Ordering::SeqCst) {
                return Err(CLOSED.to_owned());
            }
            if pending.contains_key(&call_id) {
                return Err(REPEATED_CALL.to_owned());
            }
            pending.insert(call_id.clone(), sender);
        }
        self.events.write(&Event::ToolCall {
            call_id,
            name: name.to_owned(),
            arguments: minify_json(arguments),
        });
        receiver.recv().unwrap_or_else(|_| Err(CLOSED.to_owned()))
    }
}

/// What a connection sends: one JSON line.
#[derive(Deserialize)]
struct BridgeRequest {
    #[serde(default)]
    call_id: Option<Value>,
    #[serde(default)]
    name: Option<Value>,
    #[serde(default)]
    arguments: Option<Box<RawValue>>,
}

/// One line of at most [`MAX_BRIDGE_MESSAGE_BYTES`], without its newline;
/// `None` when the other end closed first or sent more than the cap.
pub(crate) fn read_line_capped(stream: &mut impl Read) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        body.extend_from_slice(&chunk[..read]);
        if body.len() > MAX_BRIDGE_MESSAGE_BYTES {
            return None;
        }
        if let Some(newline) = body.iter().position(|byte| *byte == b'\n') {
            body.truncate(newline);
            return Some(body);
        }
    }
}

/// Answer one connection: a request line in, an `output` or `error` line
/// out. A line that is too long or not JSON closes the connection unanswered.
fn serve_connection(shared: &Shared, mut stream: impl Read + Write) {
    let Some(line) = read_line_capped(&mut stream) else {
        return;
    };
    let Ok(request) = serde_json::from_slice::<BridgeRequest>(&line) else {
        return;
    };
    let reply = match socket_request(shared, &request) {
        Ok(output) => serde_json::json!({ "output": output }),
        // `String(error)` of a JavaScript Error.
        Err(message) => serde_json::json!({ "error": format!("Error: {message}") }),
    };
    if let Ok(mut line) = serde_json::to_vec(&reply) {
        line.push(b'\n');
        let _ = stream.write_all(&line);
        let _ = stream.flush();
    }
}

fn socket_request(shared: &Shared, request: &BridgeRequest) -> Result<Value, String> {
    let Some(Value::String(name)) = &request.name else {
        return Err(OUTSIDE_CATALOG.to_owned());
    };
    if !shared.allowed.contains(name) {
        return Err(OUTSIDE_CATALOG.to_owned());
    }
    let call_id = match &request.call_id {
        None | Some(Value::Null) => None,
        Some(Value::String(call_id)) => Some(call_id.as_str()),
        Some(_) => return Err("Provider sent an invalid tool call id.".to_owned()),
    };
    let Some(arguments) = &request.arguments else {
        return Err("Provider sent no tool arguments.".to_owned());
    };
    shared.request(name, arguments, call_id)
}

/// `NativeToolBridge`: the broker's end of the tool channel for one turn.
pub struct ToolBridge {
    shared: Arc<Shared>,
    socket_path: String,
    directory: PathBuf,
    listener: platform::Listener,
    _tracked: TrackedDirectory,
}

impl ToolBridge {
    /// `NativeToolBridge.start(tools)`: listen in a fresh private directory
    /// under the system temporary directory (short enough for a macOS
    /// socket path); only `tools`' names are answered.
    pub fn start(tools: &[BrokerToolDefinition], events: EventSink) -> Result<Self, BrokerFailure> {
        let directory = make_temp_dir(&std::env::temp_dir(), "kassiber-ai-bridge-")?;
        let tracked = track_directory(&directory);
        let shared = Arc::new(Shared {
            allowed: tools.iter().map(|tool| tool.name.clone()).collect(),
            pending: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            events,
        });
        let started = platform::listen(&directory, Arc::clone(&shared));
        match started {
            Ok((socket_path, listener)) => Ok(Self {
                shared,
                socket_path,
                directory,
                listener,
                _tracked: tracked,
            }),
            Err(failure) => {
                let _ = std::fs::remove_dir_all(&directory);
                Err(failure)
            }
        }
    }

    /// The socket path (a pipe name on Windows) the MCP child connects to.
    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }

    /// `bridge.request(name, args, callId)`: ask the daemon to run an
    /// advertised tool and wait for its output. Writes the `tool_call` event;
    /// [`ToolBridge::resolve`] answers it. Without a call id a random one is
    /// used. Adapters whose CLI calls tools directly (Codex's dynamic tools)
    /// use this instead of the MCP child.
    pub fn request(
        &self,
        name: &str,
        arguments: &RawValue,
        call_id: Option<&str>,
    ) -> Result<Value, BrokerFailure> {
        self.shared
            .request(name, arguments, call_id)
            .map_err(BrokerFailure::new)
    }

    /// `bridge.resolve(results)`: the daemon's `tool_results`. Each result
    /// answers the pending call with its `call_id`; others are ignored.
    pub fn resolve(&self, results: &[Value]) {
        for result in results {
            let Some(call_id) = result.get("call_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(sender) = self.shared.pending().remove(call_id) else {
                continue;
            };
            let output = result.get("output").cloned().unwrap_or(Value::Null);
            let _ = sender.send(Ok(output));
        }
    }

    /// `bridge.close()`: refuse every pending call, stop listening and
    /// remove the directory.
    pub fn close(&self) -> Result<(), BrokerFailure> {
        if self.shared.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        for (_, sender) in self.shared.pending().drain() {
            let _ = sender.send(Err(CLOSED.to_owned()));
        }
        self.listener.stop(&self.socket_path);
        match std::fs::remove_dir_all(&self.directory) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                Err(fs_failure("rm", &self.directory, &error))
            }
            _ => Ok(()),
        }
    }
}

impl Drop for ToolBridge {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// `mcpCommand(cwd, tools, bridge)`: write the tool manifest (0600) into the
/// turn's directory and return the MCP child's command line: this
/// executable with `--provider-broker-mcp <socket> <manifest>`.
pub fn mcp_command(
    cwd: &Path,
    tools: &[BrokerToolDefinition],
    bridge: &ToolBridge,
) -> Result<Vec<String>, BrokerFailure> {
    let manifest = cwd.join(MANIFEST_NAME);
    let body = serde_json::to_vec(tools)
        .map_err(|_| BrokerFailure::new("Kassiber could not write the tool manifest."))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(&manifest)
        .and_then(|mut file| file.write_all(&body))
        .map_err(|error| fs_failure("open", &manifest, &error))?;
    let executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
        .ok_or_else(|| BrokerFailure::new("Kassiber broker executable path is unavailable."))?;
    let manifest = manifest
        .into_os_string()
        .into_string()
        .map_err(|_| BrokerFailure::new("Kassiber tool manifest path is not valid text."))?;
    Ok(vec![
        executable,
        "--provider-broker-mcp".to_owned(),
        bridge.socket_path().to_owned(),
        manifest,
    ])
}

#[cfg(unix)]
mod platform {
    use super::*;

    /// The accept thread owns the listener; stopping wakes it with one
    /// connection after the closed flag is set.
    pub struct Listener;

    pub fn listen(
        directory: &Path,
        shared: Arc<Shared>,
    ) -> Result<(String, Listener), BrokerFailure> {
        let path = directory.join("bridge.sock");
        let socket_path = path
            .to_str()
            .ok_or_else(|| BrokerFailure::new("Kassiber tool bridge path is not valid text."))?
            .to_owned();
        let listener = kassiber_ipc::unix::bind_listener(&path, 0o600, 64)
            .map_err(|error| fs_failure("listen", &path, &error))?;
        std::thread::Builder::new()
            .name("tool-bridge".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if shared.closed.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    };
                    let shared = Arc::clone(&shared);
                    let _ = std::thread::Builder::new()
                        .name("tool-bridge-call".into())
                        .spawn(move || serve_connection(&shared, stream));
                }
            })
            .map_err(|_| BrokerFailure::new("Kassiber could not start the tool bridge."))?;
        Ok((socket_path, Listener))
    }

    impl Listener {
        pub fn stop(&self, socket_path: &str) {
            let _ = kassiber_ipc::unix::connect(
                Path::new(socket_path),
                Some(std::time::Duration::from_secs(1)),
            );
        }
    }

    /// The MCP child's end: one connection per call.
    pub fn connect(socket_path: &str) -> io::Result<impl Read + Write> {
        kassiber_ipc::unix::connect(Path::new(socket_path), None)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use kassiber_ipc::windows::pipe::{self, PipeListener};

    pub struct Listener {
        pipe: Arc<PipeListener>,
    }

    pub fn listen(
        _directory: &Path,
        shared: Arc<Shared>,
    ) -> Result<(String, Listener), BrokerFailure> {
        let name = format!(r"\\.\pipe\kassiber-tool-bridge-{}", random_uuid()?);
        let sid = kassiber_ipc::windows::current_sid()
            .map_err(|_| BrokerFailure::new("Kassiber could not identify the current user."))?;
        let pipe = Arc::new(
            PipeListener::bind(&name, &sid)
                .map_err(|_| BrokerFailure::new("Kassiber could not start the tool bridge."))?,
        );
        let accepting = Arc::clone(&pipe);
        std::thread::Builder::new()
            .name("tool-bridge".into())
            .spawn(move || loop {
                let accepted = accepting.accept(&sid, None);
                if shared.closed.load(Ordering::SeqCst) {
                    break;
                }
                // A listener that fails outside a close stops answering.
                let Ok(accepted) = accepted else { break };
                if accepted.sid != sid {
                    accepted.stream.reject();
                    continue;
                }
                let shared = Arc::clone(&shared);
                let _ = std::thread::Builder::new()
                    .name("tool-bridge-call".into())
                    .spawn(move || serve_connection(&shared, accepted.stream));
            })
            .map_err(|_| BrokerFailure::new("Kassiber could not start the tool bridge."))?;
        Ok((name, Listener { pipe }))
    }

    impl Listener {
        pub fn stop(&self, _socket_path: &str) {
            self.pipe.close();
        }
    }

    /// The MCP child's end: the pipe must belong to this user.
    pub fn connect(socket_path: &str) -> io::Result<impl Read + Write> {
        let (stream, server_sid) =
            pipe::connect(socket_path, std::time::Duration::from_secs(5), None)?;
        if server_sid != kassiber_ipc::windows::current_sid()? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the tool bridge belongs to another user",
            ));
        }
        Ok(stream)
    }
}

pub(crate) use platform::connect;

#[cfg(test)]
mod tests;
