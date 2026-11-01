use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::{Book, BookContext, StatusFacts};
use kassiber_store::rusqlite::Connection;

type Result<T> = std::result::Result<T, AppError>;
pub struct ReportBook(pub Connection);
fn unexpected<T>() -> Result<T> {
    Err(AppError::new(
        "internal_error",
        "Unexpected fixture book operation",
    ))
}
impl Book for ReportBook {
    fn reports(&mut self, command: &str, args: &Value) -> Result<Value> {
        kassiber_reports::execute(&self.0, command, args)
    }
    fn is_initialized(&self) -> Result<bool> {
        Ok(true)
    }
    fn context(&self) -> Result<BookContext> {
        unexpected()
    }
    fn status_facts(&self) -> Result<StatusFacts> {
        unexpected()
    }
    fn resolve_scope(&self, _: Option<&Value>, _: Option<&Value>) -> Result<(String, String)> {
        unexpected()
    }
    fn rollback(&mut self) -> Result<()> {
        unexpected()
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
