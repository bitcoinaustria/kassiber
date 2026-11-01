//! The desktop backup transport adapter. Domain decisions live in backup.
use crate::{
    output::{with_request_id, RequestId},
    Daemon, Handled,
};
use kassiber_backup::desktop::{Fingerprint, Host};
use kassiber_envelope::{dict, s, success, type_name, AppError};
use kassiber_pyjson::Value;
use std::path::Path;

pub use kassiber_backup::desktop::KINDS;

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request: &RequestId,
) -> Handled {
    let empty = dict::<&str>([]);
    let args = match args {
        None | Some(Value::Null) => &empty,
        Some(v @ Value::Dict(_)) => v,
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(v)))]))
                .into())
        }
    };
    let mut sessions = std::mem::take(&mut daemon.backup_sessions);
    let result = sessions.handle(daemon, kind, args);
    daemon.backup_sessions = sessions;
    Ok((
        Some(with_request_id(success(kind, result?), request)),
        false,
    ))
}

impl Host for Daemon {
    fn data_root(&self) -> &Path {
        Path::new(&self.data_root)
    }
    fn unlocked(&self) -> bool {
        self.book.is_some()
    }
    fn fingerprint(&self) -> Result<Fingerprint, AppError> {
        self.book
            .as_ref()
            .expect("unlocked backup")
            .backup_fingerprint(self.project_id.as_deref())
    }
    fn summary(&self) -> Result<Value, AppError> {
        self.book
            .as_ref()
            .expect("unlocked backup")
            .backup_summary()
    }
    fn export(&mut self, output: &Path, passphrase: &str) -> Result<Value, AppError> {
        self.book.as_ref().expect("unlocked backup").backup_export(
            output,
            passphrase,
            self.build.version,
        )
    }
    fn ensure_owner(&mut self) -> Result<(), AppError> {
        Daemon::ensure_owner(self)
    }
    fn maintenance(&self) -> Result<Box<dyn Send>, AppError> {
        let lease = self.store.restore_maintenance(&self.data_root)?;
        // Workers must exit before the replacement; admission stays closed.
        let quiesced = self.analysis.quiesce(std::time::Duration::from_secs(5))?;
        Ok(Box::new((lease, quiesced)))
    }
    fn close_database(&mut self, expected: &Fingerprint) -> Result<(), AppError> {
        self.analysis.clear();
        self.book
            .as_mut()
            .expect("unlocked backup")
            .close_for_restore(expected, self.project_id.as_deref())?;
        self.book = None;
        self.unlocked_passphrase = None;
        Ok(())
    }
    fn invalidate_credentials(&mut self) -> Result<(), AppError> {
        self.runtime_config = self
            .store
            .restore_invalidate(&self.data_root, self.project_id.as_deref())?;
        Ok(())
    }
    fn release_owner(&mut self) -> Result<(), AppError> {
        Daemon::release_owner(self)
    }
}
