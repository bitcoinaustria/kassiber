//! `daemon_chat_uploads.py` (#689): files the user pastes into the chat, or
//! drops into the browser preview.
//!
//! The native picker and a desktop drop hand the daemon a path the user chose.
//! A paste has no path: the renderer holds the bytes and sends them here in
//! chunks, each below the daemon's request line limit. They are written to an
//! owner-only file in a private temporary directory, staged like a picked file,
//! and deleted when the staging grant is released or the daemon exits. Nothing
//! is fetched and nothing reaches a provider here.
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{Key, PyStr, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const KIND: &str = "ui.chat.attachments.upload";

/// The chat picker's filter: exports the generic ledger reads, and the
/// statements and photos the document importer reads.
const CHAT_ATTACHMENT_EXTENSIONS: [&str; 10] = [
    ".csv", ".tsv", ".xlsx", ".xlsm", ".pdf", ".png", ".jpg", ".jpeg", ".webp", ".gif",
];
const MAX_UPLOAD_BYTES: i64 = 25 * 1024 * 1024;
/// A picked or dropped file is read in place; the analysis tool's own limit.
const MAX_PICKED_BYTES: u64 = 64 * 1024 * 1024;
/// Base64 of a chunk this size stays well below the request line limit.
const MAX_CHUNK_BYTES: usize = 512 * 1024;
const UPLOAD_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PENDING_UPLOADS: usize = 2;
const MAX_FILENAME_CHARS: usize = 120;
const FIELDS: [&str; 5] = [
    "upload_id",
    "filename",
    "total_bytes",
    "offset",
    "chunk_base64",
];

fn validation(message: &str) -> AppError {
    AppError::new("validation", message)
}

fn expired() -> AppError {
    AppError::new(
        "chat_attachment_upload_expired",
        "The upload is no longer available",
    )
    .with_hint("Attach the file again.")
}

/// `os.path.splitext(name)` of a bare name: the extension starts at the
/// last dot that has a non-dot character before it.
fn splitext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if name[..at].chars().any(|c| c != '.') => (&name[..at], &name[at..]),
        _ => (name, ""),
    }
}

/// `chat_attachment_filename(raw)`: the file's own name, without any
/// directory, as the chat shows it.
pub fn chat_attachment_filename(raw: Option<&Value>) -> Result<String, AppError> {
    let Some(Value::Str(raw)) = raw else {
        return Err(validation("A file name is required"));
    };
    let raw = kassiber_envelope::py_str(&Value::Str(raw.clone())).replace('\\', "/");
    let name = raw.rsplit('/').next().unwrap_or_default();
    let printable: String = name
        .chars()
        .filter(|c| kassiber_pytext::is_printable(*c as u32))
        .collect();
    let name = kassiber_pytext::strip(&printable).to_owned();
    if ["", ".", ".."].contains(&name.as_str()) {
        return Err(validation("A file name is required"));
    }
    let (stem, extension) = splitext(&name);
    let extension_lower = extension.to_lowercase();
    if !CHAT_ATTACHMENT_EXTENSIONS.contains(&extension_lower.as_str()) {
        return Err(AppError::new(
            "chat_attachment_unsupported",
            "This file type cannot be attached to the chat",
        )
        .with_hint("Attach a CSV, TSV, XLSX, PDF or image file.")
        .with_details(dict([("extension", s(extension_lower))])));
    }
    if name.chars().count() > MAX_FILENAME_CHARS {
        let keep = MAX_FILENAME_CHARS.saturating_sub(extension.chars().count());
        let stem: String = stem.chars().take(keep).collect();
        return Ok(format!("{stem}{extension}"));
    }
    Ok(name)
}

/// `chat_attachment_source(source_file)`: a picked or dropped chat file, an
/// existing file of a type the chat takes. The document importer's own check
/// admits only images and PDFs; a chat attachment is usually an export.
pub fn chat_attachment_source(source_file: &str) -> Result<PathBuf, AppError> {
    let not_found = || AppError::new("not_found", "The attached file is not available");
    let expanded = expanduser(source_file);
    let candidate = std::fs::canonicalize(&expanded).map_err(|_| not_found())?;
    let metadata = std::fs::metadata(&candidate).map_err(|_| not_found())?;
    if !metadata.is_file() {
        return Err(not_found());
    }
    let name = candidate
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    chat_attachment_filename(Some(&s(name)))?;
    if metadata.len() > MAX_PICKED_BYTES {
        return Err(AppError::new(
            "chat_attachment_too_large",
            "The file is too large to attach",
        )
        .with_details(dict([("max_bytes", Value::from(MAX_PICKED_BYTES as i64))])));
    }
    Ok(candidate)
}

/// `os.path.expanduser(path)` for the current user's `~`.
fn expanduser(path: &str) -> PathBuf {
    match (path.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            let mut expanded = home;
            expanded.push(rest);
            PathBuf::from(expanded)
        }
        _ => PathBuf::from(path),
    }
}

/// `discard_upload(path)`: delete an uploaded file and the private directory
/// that holds it.
pub fn discard_upload(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return,
    }
    if let Some(directory) = Path::new(path).parent() {
        let _ = std::fs::remove_dir(directory);
    }
}

struct Pending {
    path: String,
    filename: String,
    total_bytes: i64,
    received_bytes: i64,
    scope: (String, String, String),
    last_activity: Instant,
}

/// One finished upload.
pub struct Completed {
    pub path: String,
    pub filename: String,
    pub size_bytes: i64,
}

/// What a chunk did: progress, or the finished file.
pub enum Received {
    Progress {
        upload_id: String,
        received_bytes: i64,
    },
    Complete(Completed),
}

/// `ChatUploads`: chunked uploads in progress, in one private directory per
/// daemon, removed when the daemon ends.
#[derive(Default)]
pub struct ChatUploads {
    pending: HashMap<String, Pending>,
    root: Option<PathBuf>,
}

impl Drop for ChatUploads {
    fn drop(&mut self) {
        // `atexit.register(shutil.rmtree, root, True)`.
        if let Some(root) = self.root.take() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

fn strict_int(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Int(number)) => i64::try_from(number.clone()).ok(),
        _ => None,
    }
}

impl ChatUploads {
    /// `receive(args, workspace_id=..., profile_id=..., data_root=...)`:
    /// append one chunk.
    pub fn receive(
        &mut self,
        args: &Value,
        workspace_id: &str,
        profile_id: &str,
        data_root: &str,
    ) -> Result<Received, AppError> {
        let Value::Dict(pairs) = args else {
            return Err(validation("Unknown upload fields"));
        };
        // Request arguments come from JSON, so every key is a string.
        let mut unknown: Vec<PyStr> = pairs
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(key) if !FIELDS.iter().any(|name| key.is(name)) => Some(key.clone()),
                _ => None,
            })
            .collect();
        unknown.sort();
        if !unknown.is_empty() {
            return Err(validation("Unknown upload fields").with_details(dict([(
                "fields",
                Value::List(unknown.into_iter().map(Value::Str).collect()),
            )])));
        }
        let chunk = decode_chunk(get(args, "chunk_base64"))?;
        let offset = match strict_int(get(args, "offset")) {
            Some(offset) if offset >= 0 => offset,
            _ => return Err(validation("The upload offset is invalid")),
        };
        let scope = (
            workspace_id.to_owned(),
            profile_id.to_owned(),
            data_root.to_owned(),
        );
        let now = Instant::now();
        self.prune(now);
        let upload_id = match get(args, "upload_id") {
            None | Some(Value::Null) => {
                if offset != 0 {
                    return Err(validation("A new upload starts at offset 0"));
                }
                self.start(args, scope, now)?
            }
            Some(raw) => {
                let id = match raw {
                    Value::Str(id) => kassiber_envelope::py_str(&Value::Str(id.clone())),
                    _ => return Err(expired()),
                };
                match self.pending.get(&id) {
                    Some(upload) if upload.scope == scope => {}
                    _ => return Err(expired()),
                }
                if get(args, "filename").is_some() || get(args, "total_bytes").is_some() {
                    return Err(validation("Only the first chunk names the file"));
                }
                id
            }
        };
        let upload = self.pending.get(&upload_id).expect("pending upload");
        if offset != upload.received_bytes {
            self.abort(&upload_id);
            return Err(AppError::new(
                "chat_attachment_upload_expired",
                "Upload chunks arrived out of order",
            )
            .with_hint("Attach the file again."));
        }
        if upload.received_bytes + chunk.len() as i64 > upload.total_bytes {
            self.abort(&upload_id);
            return Err(validation("The upload is larger than announced"));
        }
        let written = std::fs::OpenOptions::new()
            .append(true)
            .open(&upload.path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, &chunk));
        if written.is_err() {
            self.abort(&upload_id);
            return Err(AppError::new(
                "chat_attachment_storage_failed",
                "Could not keep the attached file",
            )
            .retryable(true));
        }
        let upload = self.pending.get_mut(&upload_id).expect("pending upload");
        upload.received_bytes += chunk.len() as i64;
        upload.last_activity = now;
        if upload.received_bytes < upload.total_bytes {
            return Ok(Received::Progress {
                received_bytes: upload.received_bytes,
                upload_id,
            });
        }
        let upload = self.pending.remove(&upload_id).expect("pending upload");
        Ok(Received::Complete(Completed {
            path: upload.path,
            filename: upload.filename,
            size_bytes: upload.total_bytes,
        }))
    }

    /// `clear()`: abort every upload in progress.
    pub fn clear(&mut self) {
        let ids: Vec<String> = self.pending.keys().cloned().collect();
        for id in ids {
            self.abort(&id);
        }
    }

    fn start(
        &mut self,
        args: &Value,
        scope: (String, String, String),
        now: Instant,
    ) -> Result<String, AppError> {
        let filename = chat_attachment_filename(get(args, "filename"))?;
        let total = match strict_int(get(args, "total_bytes")) {
            Some(total) if total >= 1 => total,
            _ => return Err(validation("The file is empty")),
        };
        if total > MAX_UPLOAD_BYTES {
            return Err(AppError::new(
                "chat_attachment_too_large",
                "The file is too large to attach",
            )
            .with_hint("Attach a file of up to 25 MB, or choose it with the file picker.")
            .with_details(dict([("max_bytes", Value::from(MAX_UPLOAD_BYTES))])));
        }
        while self.pending.len() >= MAX_PENDING_UPLOADS {
            let oldest = self
                .pending
                .iter()
                .min_by_key(|(_, upload)| upload.last_activity)
                .map(|(id, _)| id.clone())
                .expect("pending upload");
            self.abort(&oldest);
        }
        let storage = |_| {
            AppError::new(
                "chat_attachment_storage_failed",
                "Could not keep the attached file",
            )
            .retryable(true)
        };
        let directory = private_dir(&self.root()?).map_err(storage)?;
        let path = directory.join(&filename);
        create_owner_only(&path).map_err(storage)?;
        let upload_id = crate::documents_import::token_urlsafe(16)?;
        self.pending.insert(
            upload_id.clone(),
            Pending {
                path: path.to_string_lossy().into_owned(),
                filename,
                total_bytes: total,
                received_bytes: 0,
                scope,
                last_activity: now,
            },
        );
        Ok(upload_id)
    }

    fn root(&mut self) -> Result<PathBuf, AppError> {
        match &self.root {
            Some(root) if root.is_dir() => Ok(root.clone()),
            _ => {
                let root = private_dir_named(&std::env::temp_dir(), "kassiber-chat-uploads-")
                    .map_err(|_| {
                        AppError::new(
                            "chat_attachment_storage_failed",
                            "Could not keep the attached file",
                        )
                        .retryable(true)
                    })?;
                self.root = Some(root.clone());
                Ok(root)
            }
        }
    }

    fn abort(&mut self, id: &str) {
        if let Some(upload) = self.pending.remove(id) {
            discard_upload(&upload.path);
        }
    }

    fn prune(&mut self, now: Instant) {
        let stale: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, upload)| now.duration_since(upload.last_activity) >= UPLOAD_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            self.abort(&id);
        }
    }
}

/// `tempfile.mkdtemp(prefix=..., dir=parent)`: a new owner-only directory.
fn private_dir_named(parent: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    for _ in 0..100 {
        let name = crate::documents_import::token_urlsafe(6)
            .map_err(|_| std::io::Error::other("random"))?;
        let path = parent.join(format!("{prefix}{name}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("no unique directory name"))
}

fn private_dir(parent: &Path) -> std::io::Result<PathBuf> {
    private_dir_named(parent, "tmp")
}

/// `os.open(path, O_WRONLY | O_CREAT | O_EXCL, 0o600)`.
fn create_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map(drop)
}

/// `base64.b64decode(raw, validate=True)`: the alphabet, then at most two
/// `=`; a group of two data characters needs `==`, of three needs `=`.
fn b64decode_strict(raw: &str) -> Option<Vec<u8>> {
    let data = raw.trim_end_matches('=');
    let padding = raw.len() - data.len();
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let valid_padding = match data.len() % 4 {
        0 => padding <= 2 && !(data.is_empty() && padding > 0),
        2 => padding == 2,
        3 => padding == 1,
        _ => false,
    };
    if !valid_padding {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3 + 2);
    for group in data.as_bytes().chunks(4) {
        let mut bits = 0u32;
        for byte in group {
            bits = bits << 6 | value(*byte)?;
        }
        bits <<= 6 * (4 - group.len() as u32);
        let bytes = bits.to_be_bytes();
        out.extend_from_slice(&bytes[1..group.len()]);
    }
    Some(out)
}

/// `_decode_chunk(raw)`.
fn decode_chunk(raw: Option<&Value>) -> Result<Vec<u8>, AppError> {
    let raw = match raw {
        Some(Value::Str(raw)) => kassiber_envelope::py_str(&Value::Str(raw.clone())),
        _ => String::new(),
    };
    if raw.is_empty() {
        return Err(validation("An upload chunk is required"));
    }
    // 4 base64 characters carry 3 bytes; reject before decoding anything larger.
    if raw.chars().count() > (MAX_CHUNK_BYTES / 3 + 1) * 4 {
        return Err(validation("The upload chunk is too large"));
    }
    let chunk =
        b64decode_strict(&raw).ok_or_else(|| validation("The upload chunk is not base64"))?;
    if chunk.is_empty() || chunk.len() > MAX_CHUNK_BYTES {
        return Err(validation("The upload chunk is too large"));
    }
    Ok(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: Vec<(&str, Value)>) -> Value {
        dict(pairs)
    }

    #[test]
    fn strict_base64_follows_python() {
        for (raw, want) in [
            ("QQ==", Some(&b"A"[..])),
            ("QUJD", Some(b"ABC")),
            ("QUJD=", Some(b"ABC")),
            ("QUJD==", Some(b"ABC")),
            ("QUI=", Some(b"AB")),
            ("QUJDRA==", Some(b"ABCD")),
            ("QR==", Some(b"A")),
            ("", Some(b"")),
            ("QQ", None),
            ("QQ=", None),
            ("QQ===", None),
            ("Q", None),
            ("Q==", None),
            ("QUI", None),
            ("QUI==", None),
            ("=", None),
            ("Q Q==", None),
            ("QQ==QQ==", None),
            ("-_", None),
        ] {
            assert_eq!(b64decode_strict(raw).as_deref(), want, "{raw}");
        }
    }

    #[test]
    fn filenames_keep_the_name_and_the_chat_types_only() {
        let name = |raw: &str| chat_attachment_filename(Some(&s(raw)));
        assert_eq!(name("C:\\Users\\me\\Kraken.CSV").unwrap(), "Kraken.CSV");
        assert_eq!(name(" a\u{7}b.pdf ").unwrap(), "ab.pdf");
        assert_eq!(
            name("dir/.csv").unwrap_err().code,
            "chat_attachment_unsupported"
        );
        assert_eq!(
            name("notes.txt").unwrap_err().details,
            Some(dict([("extension", s(".txt"))]))
        );
        assert_eq!(name("..").unwrap_err().message, "A file name is required");
        assert_eq!(
            chat_attachment_filename(Some(&Value::from(1)))
                .unwrap_err()
                .code,
            "validation"
        );
        let long = format!("{}.xlsx", "x".repeat(200));
        let kept = name(&long).unwrap();
        assert_eq!(kept.chars().count(), MAX_FILENAME_CHARS);
        assert!(kept.ends_with(".xlsx"));
    }

    #[test]
    fn chunks_arrive_in_order_into_an_owner_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let mut uploads = ChatUploads::default();
        let first = uploads
            .receive(
                &args(vec![
                    ("filename", s("paste.csv")),
                    ("total_bytes", Value::from(4)),
                    ("offset", Value::from(0)),
                    ("chunk_base64", s("YWI=")),
                ]),
                "w",
                "p",
                "/root",
            )
            .unwrap();
        let Received::Progress {
            upload_id,
            received_bytes,
        } = first
        else {
            panic!()
        };
        assert_eq!(received_bytes, 2);
        let id = s(upload_id.as_str());
        // Another book cannot continue it.
        let foreign = uploads.receive(
            &args(vec![
                ("upload_id", id.clone()),
                ("offset", Value::from(2)),
                ("chunk_base64", s("Y2Q=")),
            ]),
            "w",
            "q",
            "/root",
        );
        assert_eq!(
            foreign.err().unwrap().code,
            "chat_attachment_upload_expired"
        );
        let done = uploads
            .receive(
                &args(vec![
                    ("upload_id", id),
                    ("offset", Value::from(2)),
                    ("chunk_base64", s("Y2Q=")),
                ]),
                "w",
                "p",
                "/root",
            )
            .unwrap();
        let Received::Complete(done) = done else {
            panic!()
        };
        assert_eq!(std::fs::read(&done.path).unwrap(), b"abcd");
        let mode = std::fs::metadata(&done.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let directory = Path::new(&done.path).parent().unwrap().to_owned();
        let root = directory.parent().unwrap().to_owned();
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        discard_upload(&done.path);
        assert!(!directory.exists());
        drop(uploads);
        assert!(!root.exists(), "the daemon's upload root ends with it");
    }

    #[test]
    fn out_of_order_or_oversized_chunks_end_the_upload() {
        let mut uploads = ChatUploads::default();
        let start = |uploads: &mut ChatUploads, total: i64| {
            let Received::Progress { upload_id, .. } = uploads
                .receive(
                    &args(vec![
                        ("filename", s("a.pdf")),
                        ("total_bytes", Value::from(total)),
                        ("offset", Value::from(0)),
                        ("chunk_base64", s("YQ==")),
                    ]),
                    "w",
                    "p",
                    "/r",
                )
                .unwrap()
            else {
                panic!()
            };
            s(upload_id.as_str())
        };
        let id = start(&mut uploads, 3);
        let error = uploads
            .receive(
                &args(vec![
                    ("upload_id", id.clone()),
                    ("offset", Value::from(5)),
                    ("chunk_base64", s("YQ==")),
                ]),
                "w",
                "p",
                "/r",
            )
            .err()
            .unwrap();
        assert_eq!(error.message, "Upload chunks arrived out of order");
        assert!(uploads.pending.is_empty());
        let id = start(&mut uploads, 2);
        let error = uploads
            .receive(
                &args(vec![
                    ("upload_id", id),
                    ("offset", Value::from(1)),
                    ("chunk_base64", s("YWI=")),
                ]),
                "w",
                "p",
                "/r",
            )
            .err()
            .unwrap();
        assert_eq!(error.message, "The upload is larger than announced");
        // Only two uploads wait at once: a third evicts the oldest.
        let first = start(&mut uploads, 9);
        start(&mut uploads, 9);
        start(&mut uploads, 9);
        assert_eq!(uploads.pending.len(), 2);
        assert!(!uploads
            .pending
            .contains_key(&kassiber_envelope::py_str(&first)));
        for (fields, message) in [
            (vec![("bogus", Value::from(1))], "Unknown upload fields"),
            (
                vec![("offset", Value::from(0))],
                "An upload chunk is required",
            ),
            (
                vec![("offset", Value::Bool(false)), ("chunk_base64", s("YQ=="))],
                "The upload offset is invalid",
            ),
            (
                vec![("offset", Value::from(0)), ("chunk_base64", s("Y"))],
                "The upload chunk is not base64",
            ),
            (
                vec![
                    ("offset", Value::from(0)),
                    ("chunk_base64", s("YQ==")),
                    ("filename", s("a.csv")),
                    ("total_bytes", Value::from(MAX_UPLOAD_BYTES + 1)),
                ],
                "The file is too large to attach",
            ),
        ] {
            assert_eq!(
                uploads
                    .receive(&args(fields), "w", "p", "/r")
                    .err()
                    .unwrap()
                    .message,
                message
            );
        }
    }
}
