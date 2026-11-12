//! Low-level publication of already prepared report bytes. This does not complete
//! an export: callers must retain their admitted book snapshot through the
//! saved-marker/dependency transaction before enabling a command or kind.
use crate::artifact_csv::Prepared;
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct PublishedFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub rows: usize,
    pub sheet: String,
}

/// Which step of a publication failed, so callers can type storage failures
/// against the artifact they named.
#[derive(Debug)]
pub enum Failure {
    /// The prepared artifacts do not fit the request; nothing was touched.
    Shape(io::Error),
    /// Writing this artifact (or creating its parents) failed.
    Write(PathBuf, io::Error),
    /// Describing the written artifact failed.
    Read(PathBuf, io::Error),
}
impl Failure {
    pub fn into_io(self) -> io::Error {
        match self {
            Self::Shape(error) | Self::Write(_, error) | Self::Read(_, error) => error,
        }
    }
}

/// Publish a single report to an already expanded user-selected path.
pub fn file(prepared: &Prepared, path: &Path) -> Result<PublishedFile, Failure> {
    let [file] = prepared.files.as_slice() else {
        return Err(Failure::Shape(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected one CSV artifact",
        )));
    };
    publish(file, path)
}

/// Publish the Austrian bundle in renderer order, matching Python's per-file
/// publication: each file creates its own parents, so a directory failure
/// names the first file. A later failure leaves earlier complete files in
/// place; it never claims a saved snapshot or a whole-directory atomic
/// transaction.
pub fn bundle(prepared: &Prepared, directory: &Path) -> Result<Vec<PublishedFile>, Failure> {
    // Rendered names must be basenames, never caller-selected relative paths.
    for file in &prepared.files {
        if Path::new(&file.name).file_name().and_then(|p| p.to_str()) != Some(&file.name)
            || file.name.contains(['/', '\\'])
        {
            return Err(Failure::Shape(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a CSV artifact basename",
            )));
        }
    }
    prepared
        .files
        .iter()
        .map(|file| publish(file, &directory.join(&file.name)))
        .collect()
}

fn publish(file: &crate::artifact_csv::File, path: &Path) -> Result<PublishedFile, Failure> {
    kassiber_store::layout::write_report_csv(path, &file.bytes)
        .map_err(|error| Failure::Write(path.to_owned(), error))?;
    let read = |error| Failure::Read(path.to_owned(), error);
    Ok(PublishedFile {
        path: resolve(path).map_err(read)?,
        bytes: path.metadata().map_err(read)?.len(),
        rows: file.rows,
        sheet: file.sheet.clone(),
    })
}

/// `Path.resolve()` (`os.path.realpath(strict=False)`): symlinks are followed
/// as far as they exist, so a device or pipe target such as `/dev/stdout`
/// still names a path.
pub fn resolve(path: &Path) -> io::Result<PathBuf> {
    use std::collections::BTreeMap;
    fn join(
        mut base: PathBuf,
        rest: &Path,
        seen: &mut BTreeMap<PathBuf, Option<PathBuf>>,
    ) -> io::Result<(PathBuf, bool)> {
        use std::path::Component;
        let mut components = rest.components();
        if rest.has_root() {
            base = PathBuf::from("/");
        }
        while let Some(component) = components.next() {
            let name = match component {
                Component::Normal(name) => name,
                Component::ParentDir => {
                    // `split(path)`; a leading run of ".." grows instead.
                    if base.as_os_str().is_empty() {
                        base = PathBuf::from("..");
                    } else if base.ends_with("..") {
                        base.pop();
                        base.push("..");
                        base.push("..");
                    } else {
                        base.pop();
                    }
                    continue;
                }
                _ => continue,
            };
            let candidate = base.join(name);
            let link = std::fs::symlink_metadata(&candidate)
                .is_ok_and(|metadata| metadata.file_type().is_symlink());
            if !link {
                base = candidate;
                continue;
            }
            match seen.get(&candidate) {
                Some(Some(resolved)) => {
                    base = resolved.clone();
                    continue;
                }
                Some(None) => {
                    let mut stuck = candidate;
                    stuck.push(components.as_path());
                    return Ok((stuck, false));
                }
                None => {}
            }
            seen.insert(candidate.clone(), None);
            let (resolved, ok) = join(base, &std::fs::read_link(&candidate)?, seen)?;
            if !ok {
                let mut stuck = resolved;
                stuck.push(components.as_path());
                return Ok((stuck, false));
            }
            seen.insert(candidate, Some(resolved.clone()));
            base = resolved;
        }
        Ok((base, true))
    }
    let (resolved, _) = join(PathBuf::new(), path, &mut BTreeMap::new())?;
    if resolved.is_absolute() {
        return Ok(resolved);
    }
    // `abspath`: anchor a relative result at the working directory.
    let mut absolute = std::env::current_dir()?;
    for component in resolved.components() {
        match component {
            std::path::Component::ParentDir => {
                absolute.pop();
            }
            std::path::Component::Normal(name) => absolute.push(name),
            _ => {}
        }
    }
    Ok(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact_csv::File;
    fn prepared() -> Prepared {
        Prepared {
            files: ["one.csv", "two.csv"]
                .map(|name| File {
                    name: name.into(),
                    sheet: name.into(),
                    rows: 3,
                    bytes: b"complete\r\n".to_vec(),
                })
                .into(),
            metadata: kassiber_pyjson::Value::Null,
            completion: None,
        }
    }
    #[test]
    fn csv_bundle_failure_retains_only_complete_preceding_files() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepared();
        std::fs::create_dir(root.path().join("two.csv")).unwrap();
        assert!(bundle(&prepared, root.path()).is_err());
        assert_eq!(
            std::fs::read(root.path().join("one.csv")).unwrap(),
            b"complete\r\n"
        );
        assert!(root.path().join("two.csv").is_dir());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn csv_publication_validates_shape_before_filesystem_changes() {
        let root = tempfile::tempdir().unwrap();
        let mut prepared = prepared();
        let path = root.path().join("new/report.csv");
        assert!(file(&prepared, &path).is_err());
        assert!(!path.parent().unwrap().exists());
        prepared.files[1].name = "../escape.csv".into();
        assert!(bundle(&prepared, path.parent().unwrap()).is_err());
        assert!(!path.parent().unwrap().exists());
    }
    /// `Path.resolve()` keeps a pipe's `pipe:[inode]` link target and
    /// anchors missing relative components at the working directory.
    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_follows_links_as_far_as_python_realpath() {
        use std::os::fd::AsRawFd;
        let (reader, _writer) = std::io::pipe().unwrap();
        let fd = Path::new("/proc/self/fd").join(reader.as_raw_fd().to_string());
        let pipe = std::fs::read_link(&fd).unwrap();
        assert!(pipe.to_string_lossy().starts_with("pipe:["));
        assert!(fd.canonicalize().is_err());
        assert_eq!(
            resolve(&fd).unwrap(),
            Path::new("/proc")
                .join(std::process::id().to_string())
                .join("fd")
                .join(&pipe)
        );
        assert_eq!(
            resolve(Path::new("missing/../x/./y")).unwrap(),
            std::env::current_dir().unwrap().join("x/y")
        );
        let root = tempfile::tempdir().unwrap();
        let real = root.path().canonicalize().unwrap();
        std::os::unix::fs::symlink("absent/target.csv", real.join("dangling.csv")).unwrap();
        assert_eq!(
            resolve(&real.join("dangling.csv")).unwrap(),
            real.join("absent/target.csv")
        );
        std::os::unix::fs::symlink("loop.csv", real.join("loop.csv")).unwrap();
        assert_eq!(
            resolve(&real.join("loop.csv")).unwrap(),
            real.join("loop.csv")
        );
    }
}
