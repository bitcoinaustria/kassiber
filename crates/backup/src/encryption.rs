//! Passphrase mode uses the same age library family as pyrage. Recipient mode
//! retains Python's PATH discovery and exact age/rage invocation. This is a
//! fixed reviewed child, never a shell, plugin, provider, or Python fallback.
use crate::{io_error, Result};
use kassiber_envelope::{py_str, s, AppError};
use kassiber_pyjson::Value;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) enum Backend {
    Passphrase,
    Binary { path: PathBuf, flavor: &'static str },
}
impl Backend {
    pub fn flavor(&self) -> &'static str {
        match self {
            // Historical wire field: denotes the in-process age format used
            // by the reference, not the name of a Rust dependency.
            Self::Passphrase => "pyrage",
            Self::Binary { flavor, .. } => flavor,
        }
    }
}

pub(crate) fn backend(passphrase: bool) -> Result<Backend> {
    if passphrase {
        return Ok(Backend::Passphrase);
    }
    let paths = std::env::var_os("PATH").unwrap_or_else(|| {
        if cfg!(windows) {
            ".;C:\\bin".into()
        } else {
            "/bin:/usr/bin".into()
        }
    });
    for flavor in ["age", "rage"] {
        let extensions: Vec<String> = if cfg!(windows) {
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WS;.MSC".into())
                .split(';')
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .collect()
        } else {
            vec![String::new()]
        };
        for directory in std::env::split_paths(&paths) {
            for extension in &extensions {
                let path = directory.join(format!("{flavor}{extension}"));
                if !path.is_file() {
                    continue;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if !fs::metadata(&path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0) {
                        continue;
                    }
                }
                return Ok(Backend::Binary { path, flavor });
            }
        }
    }
    Err(AppError::new(
        "age_unavailable",
        "recipient-mode backups require an age or rage binary",
    )
    .with_hint(
        "Install `age` or `rage` on PATH, or use passphrase-mode backups with pyrage installed.",
    ))
}

fn run(path: &Path, args: &[String], source: &Path, destination: &File) -> Result<()> {
    let status = std::process::Command::new(path)
        .args(args)
        .stdin(File::open(source).map_err(io_error)?)
        .stdout(destination.try_clone().map_err(io_error)?)
        .status()
        .map_err(io_error)?;
    if status.success() {
        return Ok(());
    }
    let command = py_str(&Value::List(
        std::iter::once(s(path.to_string_lossy().as_ref()))
            .chain(args.iter().map(|x| s(x.as_str())))
            .collect(),
    ));
    Err(AppError::new(
        "internal_error",
        format!(
            "Command '{command}' returned non-zero exit status {}.",
            status.code().unwrap_or(-1)
        ),
    ))
}

pub(crate) fn encrypt(
    backend: &Backend,
    source: &Path,
    destination: &mut File,
    passphrase: Option<&str>,
    recipients: Option<&[String]>,
) -> Result<()> {
    match backend {
        Backend::Passphrase => {
            let encryptor =
                age::Encryptor::with_user_passphrase(passphrase.expect("mode").to_owned().into());
            let mut writer = encryptor.wrap_output(destination).map_err(io_error)?;
            io::copy(&mut File::open(source).map_err(io_error)?, &mut writer).map_err(io_error)?;
            writer.finish().map_err(io_error)?;
            Ok(())
        }
        Backend::Binary { path, .. } => {
            let args = recipients
                .unwrap_or_default()
                .iter()
                .flat_map(|r| ["--recipient".to_owned(), r.clone()])
                .collect::<Vec<_>>();
            run(path, &args, source, destination)
        }
    }
}

pub(crate) fn decrypt(
    backend: &Backend,
    source: &Path,
    destination: &mut File,
    passphrase: Option<&str>,
    identity: Option<&Path>,
) -> Result<()> {
    match backend {
        Backend::Passphrase => {
            let fail = |e: age::DecryptError| {
                AppError::new(
                    "age_decrypt_failed",
                    format!("age passphrase decryption failed: {e}"),
                )
            };
            let decryptor =
                age::Decryptor::new(File::open(source).map_err(io_error)?).map_err(fail)?;
            let identity = age::scrypt::Identity::new(passphrase.expect("mode").to_owned().into());
            let mut reader = decryptor
                .decrypt(std::iter::once(&identity as &dyn age::Identity))
                .map_err(fail)?;
            io::copy(&mut reader, destination).map_err(|e| {
                AppError::new(
                    "age_decrypt_failed",
                    format!("age passphrase decryption failed: {e}"),
                )
            })?;
            destination.flush().map_err(io_error)?;
            Ok(())
        }
        Backend::Binary { path, .. } => run(
            path,
            &[
                "--decrypt".into(),
                "--identity".into(),
                identity.expect("mode").to_string_lossy().into_owned(),
            ],
            source,
            destination,
        ),
    }
}
