//! Low-level publication of already prepared CSV bytes. This does not complete
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

/// Publish a single report to an already expanded user-selected path.
pub fn file(prepared: &Prepared, path: &Path) -> io::Result<PublishedFile> {
    let [file] = prepared.files.as_slice() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected one CSV artifact",
        ));
    };
    publish(file, path)
}

/// Publish the Austrian bundle in renderer order, matching Python's per-file
/// publication. A later failure leaves earlier complete files in place; it
/// never claims a saved snapshot or a whole-directory atomic transaction.
pub fn bundle(prepared: &Prepared, directory: &Path) -> io::Result<Vec<PublishedFile>> {
    // Rendered names must be basenames, never caller-selected relative paths.
    for file in &prepared.files {
        if Path::new(&file.name).file_name().and_then(|p| p.to_str()) != Some(&file.name)
            || file.name.contains(['/', '\\'])
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a CSV artifact basename",
            ));
        }
    }
    std::fs::create_dir_all(directory)?;
    prepared
        .files
        .iter()
        .map(|file| publish(file, &directory.join(&file.name)))
        .collect()
}

fn publish(file: &crate::artifact_csv::File, path: &Path) -> io::Result<PublishedFile> {
    kassiber_store::layout::write_report_csv(path, &file.bytes)?;
    Ok(PublishedFile {
        path: path.canonicalize()?,
        bytes: path.metadata()?.len(),
        rows: file.rows,
        sheet: file.sheet.clone(),
    })
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
}
