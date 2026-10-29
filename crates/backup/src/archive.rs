//! Inspect every logical tar member before extracting any payload. Do not use
//! tar::Entry::unpack: Python normalizes paths and allows only these types.
use crate::{io_error, Result};
use kassiber_envelope::{dict, s, AppError};
use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

const MAX_MEMBER: u64 = 4 * 1024 * 1024 * 1024;

pub(crate) fn finish(mut tar: tar::Builder<File>) -> Result<()> {
    tar.finish().map_err(io_error)?;
    let tar = tar.into_inner().map_err(io_error)?;
    // Python tarfile closes on a twenty-block record boundary.
    let size = tar.metadata().map_err(io_error)?.len();
    tar.set_len(size.div_ceil(10240) * 10240).map_err(io_error)
}

fn py_name(mut bytes: &[u8]) -> kassiber_pyjson::PyStr {
    // CPython tarfile's UTF-8 + surrogateescape decoding.
    let mut points = Vec::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(valid) => {
                points.extend(valid.chars().map(u32::from));
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                points.extend(
                    std::str::from_utf8(&bytes[..valid])
                        .expect("valid prefix")
                        .chars()
                        .map(u32::from),
                );
                let end = valid + error.error_len().unwrap_or(bytes.len() - valid);
                points.extend(bytes[valid..end].iter().map(|b| 0xdc00 + u32::from(*b)));
                bytes = &bytes[end..];
            }
        }
    }
    kassiber_pyjson::PyStr::from_code_points(points).expect("Unicode or surrogateescape")
}
fn unsafe_member(name: &[u8], reason: impl Into<String>) -> AppError {
    let reason = reason.into();
    AppError::new(
        "unsafe_backup_member",
        format!("backup contains an unsafe tar member: {reason}"),
    )
    .with_details(dict([("member", s(py_name(name))), ("reason", s(reason))]))
}
fn normalize(name: &[u8]) -> Result<Vec<u8>> {
    for (refused, reason) in [
        (name.starts_with(b"/"), "absolute path"),
        (name.contains(&b'\\'), "backslash in path (Windows-style)"),
        (name.contains(&0), "NUL byte in path"),
    ] {
        if refused {
            return Err(unsafe_member(name, reason));
        }
    }
    let mut parts = vec![];
    for part in name.split(|b| *b == b'/') {
        match part {
            b"" | b"." => (),
            b".." => return Err(unsafe_member(name, "parent-directory traversal")),
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(unsafe_member(name, "empty path after normalization"));
    }
    Ok(parts.join(&b'/'))
}
fn inspect(name: &[u8], kind: u8, size: u64, seen: &mut HashSet<Vec<u8>>) -> Result<Vec<u8>> {
    let normalized = normalize(name)?;
    let top = normalized.split(|b| *b == b'/').next().expect("nonempty");
    if ![
        b"manifest.json".as_slice(),
        b"kassiber.sqlite3",
        b"attachments",
        b"config",
    ]
    .contains(&top)
    {
        return Err(unsafe_member(name, format!("top-level entry {} not in ['attachments', 'config', 'kassiber.sqlite3', 'manifest.json']", kassiber_pytext::repr_py(&py_name(top)))));
    }
    let reason = match kind {
        b'2' => Some("symlink"),
        b'1' => Some("hardlink"),
        b'3' | b'4' => Some("device node"),
        b'6' => Some("FIFO"),
        0 | b'0' | b'7' | b'S' | b'5' => None,
        _ => {
            let repr = if b"\t\n\r".contains(&kind) || kind.is_ascii_graphic() || kind == b' ' {
                format!(
                    "b{}",
                    kassiber_pytext::repr_str(&char::from(kind).to_string())
                )
            } else {
                format!("b'\\x{kind:02x}'")
            };
            return Err(unsafe_member(
                name,
                format!("unsupported member type {repr}"),
            ));
        }
    };
    if let Some(reason) = reason {
        return Err(unsafe_member(name, reason));
    }
    if kind != b'5' && size > MAX_MEMBER {
        return Err(unsafe_member(
            name,
            format!("declared size {size} exceeds limit {MAX_MEMBER}"),
        ));
    }
    if !seen.insert(normalized.clone()) {
        return Err(unsafe_member(name, "duplicate path"));
    }
    Ok(normalized)
}

fn member_path(name: Vec<u8>) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(std::ffi::OsString::from_vec(name).into())
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(name).map(PathBuf::from).map_err(io_error)
    }
}

pub fn extract(tarball: &Path, destination: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(File::open(tarball).map_err(io_error)?);
    let mut clean = vec![];
    let mut seen = HashSet::new();
    for entry in archive.entries().map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let bytes = entry.path_bytes();
        clean.push(inspect(
            &bytes,
            entry.header().entry_type().as_byte(),
            entry.size(),
            &mut seen,
        )?);
    }
    fs::create_dir_all(destination).map_err(io_error)?;
    let mut archive = tar::Archive::new(File::open(tarball).map_err(io_error)?);
    for (entry, name) in archive.entries().map_err(io_error)?.zip(clean) {
        let mut entry = entry.map_err(io_error)?;
        let target = destination.join(member_path(name)?);
        if entry.header().entry_type().as_byte() == b'5' {
            fs::create_dir_all(target).map_err(io_error)?;
        } else {
            fs::create_dir_all(target.parent().expect("parent")).map_err(io_error)?;
            let mut output = File::create(&target).map_err(io_error)?;
            std::io::copy(&mut entry, &mut output).map_err(io_error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = entry.header().mode().map_err(io_error)? & 0o777;
                fs::set_permissions(
                    target,
                    fs::Permissions::from_mode(if mode == 0 { 0o600 } else { mode }),
                )
                .map_err(io_error)?;
            }
        }
    }
    Ok(())
}

fn tree(root: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        paths.push(entry.path());
        if entry.file_type().map_err(io_error)?.is_dir() {
            tree(&entry.path(), paths)?;
        }
    }
    Ok(())
}
pub fn regular_count(root: &Path) -> Result<usize> {
    let mut paths = vec![];
    tree(root, &mut paths)?;
    Ok(paths.iter().filter(|p| p.is_file()).count())
}

pub(crate) fn append_tree(
    tar: &mut tar::Builder<File>,
    root: &Path,
    prefix: &str,
) -> Result<usize> {
    let mut paths = vec![];
    tree(root, &mut paths)?;
    paths.sort();
    let mut count = 0;
    for path in paths {
        let relative = path.strip_prefix(root).expect("tree member");
        let name = Path::new(prefix).join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.is_file() {
            append_file(tar, &path, &name)?;
            count += 1;
        } else if metadata.is_dir() {
            let mut header = tar::Header::new_ustar();
            header.set_metadata(&metadata);
            header.set_uid(0);
            header.set_gid(0);
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Directory);
            append_metadata(tar, &mut header, &metadata, &name, std::io::empty())?;
        }
    }
    Ok(count)
}

pub(crate) fn append_file(
    tar: &mut tar::Builder<File>,
    source: &Path,
    name: impl AsRef<Path>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(source).map_err(io_error)?;
    let mut header = tar::Header::new_ustar();
    header.set_metadata(&metadata);
    header.set_uid(0);
    header.set_gid(0);
    append_metadata(
        tar,
        &mut header,
        &metadata,
        name.as_ref(),
        File::open(source).map_err(io_error)?,
    )
}

fn append_metadata(
    tar: &mut tar::Builder<File>,
    header: &mut tar::Header,
    metadata: &fs::Metadata,
    name: &Path,
    contents: impl std::io::Read,
) -> Result<()> {
    #[cfg(unix)]
    let mut name = {
        use std::os::unix::ffi::OsStrExt;
        name.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let mut name = name.to_string_lossy().as_bytes().to_vec();
    if header.entry_type().is_dir() && !name.ends_with(b"/") {
        name.push(b'/');
    }
    #[cfg(unix)]
    let modified = {
        use std::os::unix::fs::MetadataExt;
        metadata.mtime() as f64 + metadata.mtime_nsec() as f64 / 1e9
    };
    #[cfg(not(unix))]
    let modified = metadata
        .modified()
        .map_err(io_error)?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io_error)?
        .as_secs_f64();
    // gettarinfo always stores stat.st_mtime as a float. Python's default PAX
    // format therefore emits an extended mtime record for every disk member.
    let mtime = kassiber_pyjson::dumps(
        &kassiber_pyjson::Value::Float(modified),
        &Default::default(),
    )
    .map_err(io_error)?;
    let mut pax = vec![];
    if std::str::from_utf8(&name).is_err() {
        pax.push(("hdrcharset", b"BINARY".as_slice()));
    }
    if !name.is_ascii() || name.len() > 100 {
        pax.push(("path", name.as_slice()));
    }
    pax.push(("mtime", mtime.as_bytes()));
    tar.append_pax_extensions(pax).map_err(io_error)?;
    let fallback: String = py_name(&name)
        .code_points()
        .map(|c| if c < 128 { char::from(c as u8) } else { '?' })
        .take(100)
        .collect();
    header.set_path(fallback).map_err(io_error)?;
    header.set_mtime(modified.round_ties_even().max(0.0) as u64);
    header.set_cksum();
    tar.append(header, contents).map_err(io_error)
}
pub(crate) fn append_bytes(tar: &mut tar::Builder<File>, name: &str, bytes: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io_error)?
            .as_secs(),
    );
    header.set_cksum();
    tar.append_data(&mut header, name, bytes).map_err(io_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_members_preserve_pax_names_mtimes_and_python_record_padding() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::write(&source, b"evidence").unwrap();
        let output = root.path().join("bundle.tar");
        let mut tar = tar::Builder::new(File::create(&output).unwrap());
        let mut names = vec![
            PathBuf::from("attachments/ascii"),
            PathBuf::from("attachments/₿"),
            PathBuf::from(format!("attachments/{}", "long".repeat(60))),
        ];
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            names.push(PathBuf::from(std::ffi::OsString::from_vec(
                b"attachments/raw-\xff".to_vec(),
            )));
        }
        for name in &names {
            append_file(&mut tar, &source, name).unwrap();
        }
        finish(tar).unwrap();
        assert_eq!(output.metadata().unwrap().len() % 10240, 0);
        let mut tar = tar::Archive::new(File::open(output).unwrap());
        let entries = tar.entries().unwrap();
        for (entry, name) in entries.zip(&names) {
            let mut entry = entry.unwrap();
            assert_eq!(&entry.path().unwrap(), name);
            let extensions = entry
                .pax_extensions()
                .unwrap()
                .unwrap()
                .map(|p| {
                    let p = p.unwrap();
                    (p.key_bytes().to_vec(), p.value_bytes().to_vec())
                })
                .collect::<Vec<_>>();
            assert!(extensions.iter().any(|(k, _)| k == b"mtime"));
        }
        // One short file still receives PAX metadata and full record padding.
        let short = root.path().join("short.tar");
        let mut tar = tar::Builder::new(File::create(&short).unwrap());
        append_file(&mut tar, &source, "attachments/short").unwrap();
        finish(tar).unwrap();
        assert_eq!(short.metadata().unwrap().len(), 10240);
    }

    #[test]
    fn path_types_sizes_and_normalized_duplicates_match_python() {
        for (name, kind, size, reason) in [
            ("/attachments/x", b'0', 0, "absolute path"),
            ("attachments/../x", b'0', 0, "parent-directory traversal"),
            (
                "attachments\\x",
                b'0',
                0,
                "backslash in path (Windows-style)",
            ),
            ("attachments/\0x", b'0', 0, "NUL byte in path"),
            ("./", b'0', 0, "empty path after normalization"),
            ("exports/x", b'0', 0, "top-level entry"),
            ("attachments/x", b'1', 0, "hardlink"),
            ("attachments/x", b'2', 0, "symlink"),
            ("attachments/x", b'3', 0, "device node"),
            ("attachments/x", b'4', 0, "device node"),
            ("attachments/x", b'6', 0, "FIFO"),
            ("attachments/x", b's', 0, "unsupported member type"),
            ("attachments/x", b'0', MAX_MEMBER + 1, "declared size"),
        ] {
            let error = inspect(name.as_bytes(), kind, size, &mut HashSet::new()).unwrap_err();
            assert_eq!(error.code, "unsafe_backup_member");
            assert!(error.message.contains(reason), "{error:?}");
        }
        let mut seen = HashSet::new();
        assert_eq!(
            inspect(b"./attachments//./x", b'0', MAX_MEMBER, &mut seen).unwrap(),
            b"attachments/x"
        );
        assert!(inspect(b"attachments/x", b'0', 1, &mut seen)
            .unwrap_err()
            .message
            .ends_with("duplicate path"));
        for kind in [0, b'0', b'7', b'S', b'5'] {
            inspect(b"config/x", kind, MAX_MEMBER, &mut HashSet::new()).unwrap();
        }
    }

    #[test]
    fn inspection_precedes_all_payload_writes_and_extracts_modes() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("bundle.tar");
        let mut tar = tar::Builder::new(File::create(&bundle).unwrap());
        append_bytes(&mut tar, "attachments/a", b"retained").unwrap();
        append_bytes(&mut tar, "attachments/a", b"duplicate").unwrap();
        tar.finish().unwrap();
        drop(tar);
        let output = root.path().join("output");
        assert_eq!(
            extract(&bundle, &output).unwrap_err().code,
            "unsafe_backup_member"
        );
        assert!(!output.exists());
        let mut tar = tar::Builder::new(File::create(&bundle).unwrap());
        append_bytes(&mut tar, "attachments/hello-\u{20bf}", b"exact\0\xff").unwrap();
        tar.finish().unwrap();
        drop(tar);
        extract(&bundle, &output).unwrap();
        assert_eq!(
            fs::read(output.join("attachments/hello-\u{20bf}")).unwrap(),
            b"exact\0\xff"
        );
    }
}
