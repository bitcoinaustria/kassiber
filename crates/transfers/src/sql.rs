use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::ValueRef, Connection, Params};
use std::cell::RefCell;

pub type Result<T> = std::result::Result<T, AppError>;

/// Scope is resolved by the book adapter before entering a domain service.
pub struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
}

pub fn error(error: kassiber_store::rusqlite::Error) -> AppError {
    let mapped = mapped_error(&error);
    SQLITE_ERRORS.with(|errors| {
        if let Some(slot) = errors.borrow_mut().last_mut() {
            if slot.is_none() {
                *slot = Some(error);
            }
        }
    });
    mapped
}

fn mapped_error(error: &kassiber_store::rusqlite::Error) -> AppError {
    // Python propagates trigger refusals through the service boundary. Preserve
    // their message without exposing the SQL statement or connection details.
    // The shell's shared envelope still applies output redaction.
    if let kassiber_store::rusqlite::Error::SqliteFailure(code, Some(message)) = error {
        if code.extended_code == kassiber_store::rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER {
            return AppError::new("internal_error", message.clone());
        }
    }
    let copy = match error {
        kassiber_store::rusqlite::Error::SqliteFailure(code, message) => {
            kassiber_store::rusqlite::Error::SqliteFailure(*code, message.clone())
        }
        _ => kassiber_store::rusqlite::Error::InvalidQuery,
    };
    let e = kassiber_store::map_sqlite_error(copy);
    AppError::new(e.code, e.message).retryable(e.retryable)
}

thread_local! {
    static SQLITE_ERRORS: RefCell<Vec<Option<kassiber_store::rusqlite::Error>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Debug)]
pub enum CapturedError {
    Sqlite(kassiber_store::rusqlite::Error),
    App(AppError),
    Python {
        family: &'static str,
        message: String,
    },
}

impl From<AppError> for CapturedError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

struct ErrorScope(bool);

impl Drop for ErrorScope {
    fn drop(&mut self) {
        if self.0 {
            SQLITE_ERRORS.with(|errors| {
                errors.borrow_mut().pop();
            });
        }
    }
}

pub(super) fn capture_sqlite<T>(
    f: impl FnOnce() -> std::result::Result<T, CapturedError>,
) -> std::result::Result<T, CapturedError> {
    SQLITE_ERRORS.with(|errors| errors.borrow_mut().push(None));
    let mut scope = ErrorScope(true);
    let result = f();
    let error = SQLITE_ERRORS.with(|errors| errors.borrow_mut().pop().flatten());
    scope.0 = false;
    match (result, error) {
        (Ok(value), _) => Ok(value),
        (Err(_), Some(error)) => Err(CapturedError::Sqlite(error)),
        (Err(error), None) => Err(error),
    }
}

pub fn rows(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).map_err(error)?;
    let names: Vec<_> = stmt.column_names().iter().map(|x| x.to_string()).collect();
    let mut cursor = stmt.query(params).map_err(error)?;
    let mut result = Vec::new();
    while let Some(row) = cursor.next().map_err(error)? {
        let mut pairs = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let value = match row.get_ref(i).map_err(error)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::Int(n.into()),
                ValueRef::Real(n) => Value::Float(n),
                ValueRef::Text(t) => s(std::str::from_utf8(t)
                    .map_err(|_| AppError::new("validation", "stored text is not valid UTF-8"))?),
                ValueRef::Blob(raw) => Value::Bytes(raw.to_vec()),
            };
            pairs.push((name.as_str(), value));
        }
        result.push(dict(pairs));
    }
    Ok(result)
}

/// Injectable UUID/clock source for Python row oracles; production has no overrides.
pub trait Sources {
    fn uuid(&self, conn: &Connection) -> Result<String>;
    fn now(&self, conn: &Connection) -> Result<String>;
    /// `datetime.now(timezone.utc).isoformat()` (microseconds when non-zero).
    fn now_precise(&self, conn: &Connection) -> Result<String> {
        self.now(conn)
    }
    /// A fixed instant for the reads Python takes from `datetime.now()`
    /// directly, outside the `now_iso()` seam: Python reference vectors froze
    /// that clock too. Harness builds only; the product reads the system
    /// clock there (`wall_clock`).
    #[cfg(feature = "test-harness")]
    fn pinned_wall_clock(&self) -> Option<String> {
        None
    }
}

/// `datetime.now(timezone.utc)` in whole seconds: the system clock, or in a
/// harness build the instant its sources pin.
pub(crate) fn wall_clock(conn: &Connection, sources: &dyn Sources) -> Result<String> {
    #[cfg(feature = "test-harness")]
    if let Some(now) = sources.pinned_wall_clock() {
        return Ok(now);
    }
    let _ = sources;
    SystemSources.now(conn)
}
pub struct SystemSources;
impl Sources for SystemSources {
    fn uuid(&self, conn: &Connection) -> Result<String> {
        let mut bytes: Vec<u8> = conn
            .query_row("SELECT randomblob(16)", [], |r| r.get(0))
            .map_err(error)?;
        bytes[6] = (bytes[6] & 15) | 64;
        bytes[8] = (bytes[8] & 63) | 128;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        ))
    }
    fn now(&self, conn: &Connection) -> Result<String> {
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
            r.get(0)
        })
        .map_err(error)
    }
    fn now_precise(&self, _: &Connection) -> Result<String> {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| AppError::new("internal_error", "system clock is before 1970"))?;
        Ok(iso_micros(
            elapsed.as_secs() as i64,
            elapsed.subsec_micros(),
        ))
    }
}
/// Python's UTC `isoformat()` with a `Z` suffix; microseconds only when non-zero.
pub fn iso_micros(seconds: i64, micros: u32) -> String {
    let days = seconds.div_euclid(86_400);
    let clock = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let fraction = if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}")
    };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{fraction}Z",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}
/// Own the operation savepoint; never commit another service's partial writes.
pub fn atomic<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    kassiber_store::begin_write_savepoint(conn, "transfers_service").map_err(error)?;
    match f() {
        Ok(value) => {
            if let Err(e) = conn.execute_batch("RELEASE SAVEPOINT transfers_service") {
                let _ = conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT transfers_service; RELEASE SAVEPOINT transfers_service",
                );
                return Err(error(e));
            }
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT transfers_service; RELEASE SAVEPOINT transfers_service",
            );
            Err(e)
        }
    }
}

pub fn bind(value: &Value, index: usize) -> Result<kassiber_store::rusqlite::types::Value> {
    use kassiber_store::rusqlite::types::Value as Sql;
    match value {
        Value::Null => Ok(Sql::Null),
        Value::Bool(v) => Ok(Sql::Integer(i64::from(*v))),
        Value::Int(n) => n.to_string().parse().map(Sql::Integer).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        }),
        Value::Float(v) => Ok(Sql::Real(*v)),
        Value::Str(v) => v.as_str().map(|s| Sql::Text(s.into())).ok_or_else(|| {
            AppError::new("internal_error", kassiber_pytext::utf8_error(v).unwrap())
        }),
        _ => Err(AppError::new(
            "internal_error",
            format!(
                "Error binding parameter {index}: type '{}' is not supported",
                kassiber_envelope::type_name(value)
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_sqlite_capture_preserves_errors_and_restores_scope() {
        let c = Connection::open_in_memory().unwrap();
        let mapped = error(c.execute("SELECT absent", []).unwrap_err());
        assert_eq!(mapped.code, "internal_error");
        let domain = || {
            Err::<(), _>(CapturedError::App(AppError::new(
                "validation",
                "domain refusal",
            )))
        };
        assert!(matches!(capture_sqlite(domain), Err(CapturedError::App(_))));
        let raw = capture_sqlite(|| {
            c.execute("SELECT absent", []).map_err(error)?;
            Ok(())
        });
        assert!(matches!(raw, Err(CapturedError::Sqlite(_))));
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: std::result::Result<(), CapturedError> = capture_sqlite(|| {
                let _ = c.execute("SELECT absent", []).map_err(error);
                panic!("injected capture unwind")
            });
        }));
        assert!(panic.is_err());
        assert!(matches!(capture_sqlite(domain), Err(CapturedError::App(_))));
        SQLITE_ERRORS.with(|errors| assert!(errors.borrow().is_empty()));
    }
}
