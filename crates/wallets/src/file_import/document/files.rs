use super::*;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) const MAX_SOURCE: u64 = 25 * 1024 * 1024;

pub(in crate::file_import) fn io(e: std::io::Error) -> AppError {
    AppError::new("internal_error", e.to_string())
}
pub(super) fn locator(value: &str) -> Result<(String, Option<String>), AppError> {
    let value = value
        .trim_start_matches(|c: char| c as u32 <= 32)
        .replace(['\t', '\r', '\n'], "");
    let (scheme, rest) = value.split_once(':').unwrap_or(("", &value));
    let scheme = scheme.to_ascii_lowercase();
    let Some(rest) = rest.strip_prefix("//") else {
        return Ok((scheme, None));
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    let host = if let Some(host) = authority.strip_prefix('[') {
        let Some((host, _)) = host.split_once(']') else {
            return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
        };
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(AppError::new(
                "internal_error",
                format!("'{host}' does not appear to be an IPv4 or IPv6 address"),
            ));
        }
        host
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    Ok((
        scheme,
        (!host.is_empty()).then(|| kassiber_pytext::lower(host)),
    ))
}
pub(super) fn source_path(raw: &str) -> Result<PathBuf, AppError> {
    let raw = kassiber_pytext::strip(raw);
    let (scheme, host) = locator(raw)?;
    if matches!(scheme.as_str(), "http" | "https") {
        let host = host.unwrap_or_default();
        let hint = if ["docs.google.com", "drive.google.com"].contains(&host.as_str()) {
            "Open the Google link in your logged-in browser, download the PDF/image, then import the downloaded local file. Kassiber does not read browser cookies."
        } else {
            "Download the document locally, then import the local PDF/image file."
        };
        return Err(AppError::new(
            "document_import_url_not_supported",
            "Document OCR import accepts local files, not URLs",
        )
        .with_hint(hint)
        .with_details(dict([("host", s(host))])));
    }
    let path = kassiber_paths::Context::from_env()
        .and_then(|ctx| ctx.expand_user(Path::new(if raw.is_empty() { "." } else { raw })))
        .map_err(|e| AppError::new(e.code, e.message))?;
    if !path.exists() {
        return Err(AppError::new(
            "not_found",
            format!("Document source not found: {}", path.display()),
        )
        .with_hint("Choose an existing image or PDF file."));
    }
    if !path.is_file() {
        return Err(AppError::new(
            "validation",
            "Document source must be a file",
        ));
    }
    let extension = suffix(&path);
    if ![".jpg", ".jpeg", ".png", ".webp", ".gif", ".pdf"].contains(&extension.as_str()) {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported document type '{}'",
                if extension.is_empty() {
                    "(none)"
                } else {
                    &extension
                }
            ),
        )
        .with_hint("Choose a JPEG, PNG, WebP, GIF, or PDF file.")
        .with_details(dict([(
            "supported_extensions",
            Py::List(
                [".gif", ".jpeg", ".jpg", ".pdf", ".png", ".webp"]
                    .into_iter()
                    .map(s)
                    .collect(),
            ),
        )])));
    }
    let size = fs::metadata(&path).map_err(io)?.len();
    if size > MAX_SOURCE {
        return Err(AppError::new(
            "validation",
            "Document source is too large for local OCR import",
        )
        .with_hint("Split the statement or export only the pages that contain transactions.")
        .with_details(dict([
            ("size_bytes", Py::from(size as i64)),
            ("max_bytes", Py::from(MAX_SOURCE as i64)),
        ])));
    }
    Ok(path)
}
pub(super) fn suffix(path: &Path) -> String {
    path.extension()
        .and_then(|v| v.to_str())
        .filter(|v| !v.is_empty())
        .map_or(String::new(), |v| format!(".{}", v.to_lowercase()))
}
pub(super) fn mime(path: &Path) -> &'static str {
    match suffix(path).as_str() {
        ".jpg" | ".jpeg" => "image/jpeg",
        ".png" => "image/png",
        ".webp" => "image/webp",
        ".gif" => "image/gif",
        ".pdf" => "application/pdf",
        // Python's own table; `.xlsx` and `.xlsm` are not in it.
        ".csv" => "text/csv",
        ".tsv" => "text/tab-separated-values",
        _ => "application/octet-stream",
    }
}
pub(super) fn hash(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path).map_err(io)?;
    let mut digest = Sha256::new();
    let mut bytes = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut bytes).map_err(io)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect())
}

pub(super) fn managed_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = kassiber_pytext::strip(relative);
    if relative.is_empty() || Path::new(relative).is_absolute() {
        return None;
    }
    let root = kassiber_paths::canonicalize_allow_missing(root).ok()?;
    let path = kassiber_paths::canonicalize_allow_missing(&root.join(relative)).ok()?;
    path.starts_with(root).then_some(path)
}

pub(in crate::file_import) struct Temp(PathBuf);
impl Temp {
    pub(in crate::file_import) fn new(label: &str) -> Result<Self, AppError> {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        loop {
            let id = SERIAL.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "kassiber-{label}-{}-{}-{id}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io(e)),
            }
        }
    }
    pub(in crate::file_import) fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(super) struct Snapshot {
    pub(super) path: PathBuf,
    _temp: Temp,
}
impl Snapshot {
    pub(super) fn new(path: &Path) -> Result<Self, AppError> {
        let temp = Temp::new("ocr-source")?;
        let stable = temp.path().join(path.file_name().unwrap_or_default());
        let mut input = fs::File::open(path).map_err(io)?.take(MAX_SOURCE + 1);
        let mut output = fs::File::create(&stable).map_err(io)?;
        std::io::copy(&mut input, &mut output).map_err(io)?;
        source_path(&stable.to_string_lossy())?;
        Ok(Self {
            path: stable,
            _temp: temp,
        })
    }
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        out.push(ABC[(a >> 2) as usize] as char);
        out.push(ABC[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            ABC[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ABC[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
pub(super) fn image_part(path: &Path) -> Result<Py, AppError> {
    let bytes = std::fs::read(path).map_err(files::io)?;
    Ok(dict([
        ("type", s("image_url")),
        (
            "image_url",
            dict([(
                "url",
                s(format!(
                    "data:{};base64,{}",
                    files::mime(path),
                    base64(&bytes)
                )),
            )]),
        ),
    ]))
}
