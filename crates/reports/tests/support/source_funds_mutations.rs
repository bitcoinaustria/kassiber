#[allow(dead_code)]
#[path = "source_funds_cases.rs"]
mod cases;
pub use cases::{encode, text, value};
#[allow(dead_code)]
pub fn dump(conn: &Connection) -> Value {
    cases::dump(conn)
}
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::Connection;
use std::cell::RefCell;

pub struct Fixed<'a> {
    pub case: &'a Value,
    pub calls: RefCell<Vec<Value>>,
}
impl<'a> Fixed<'a> {
    pub fn new(case: &'a Value) -> Self {
        Self {
            case,
            calls: RefCell::new(Vec::new()),
        }
    }
}
impl Sources for Fixed<'_> {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        let index = self
            .calls
            .borrow()
            .iter()
            .filter(|v| **v == s("uuid"))
            .count();
        self.calls.borrow_mut().push(s("uuid"));
        if kassiber_envelope::truthy(value(self.case, "sequential_ids")) {
            return Ok(format!("00000000-0000-4000-8000-{:012}", index + 1));
        }
        if let Value::List(ids) = value(self.case, "ids") {
            return Ok(kassiber_envelope::py_str(&ids[index]));
        }
        Ok(kassiber_envelope::get(self.case, "id")
            .map(kassiber_envelope::py_str)
            .unwrap_or_else(|| "new-source".into()))
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        self.calls.borrow_mut().push(s("now"));
        let count = self
            .calls
            .borrow()
            .iter()
            .filter(|v| **v == s("now"))
            .count();
        if value(self.case, "fail_now") == &Value::Int(count.into()) {
            return Err(AppError::new("validation", "late source clock refusal"));
        }
        Ok("2026-01-02T03:04:05Z".into())
    }
}

pub fn outcome(conn: &Connection, case: &Value) -> Value {
    let fixed = Fixed::new(case);
    let command = text(case, "command");
    let args = value(case, "args");
    let result = if command == "helper.review_context" {
        kassiber_setup::references::scope(conn, &dict::<&str>([])).and_then(|(_, profile)| {
            kassiber_reports::source_funds_review::review_context(
                conn,
                &profile,
                text(args, "target_transaction"),
                value(args, "recipe"),
            )
        })
    } else {
        kassiber_reports::execute_with_sources(conn, command, args, &fixed)
    };
    let (mut result, envelope) = match result {
        Ok(data) => (
            dict([("ok", data.clone())]),
            kassiber_envelope::success(command, data),
        ),
        Err(error) => {
            let fields = dict([
                ("code", s(error.code.clone())),
                ("message", s(error.message.clone())),
                ("hint", error.hint.clone().map(s).unwrap_or(Value::Null)),
                ("details", error.details.clone().unwrap_or(Value::Null)),
                ("retryable", Value::Bool(error.retryable)),
            ]);
            (dict([("error", fields)]), error.envelope(None))
        }
    };
    kassiber_envelope::set(&mut result, "calls", Value::List(fixed.calls.into_inner()));
    kassiber_envelope::set(
        &mut result,
        "cli",
        s(kassiber_envelope::cli_text(&envelope).unwrap()),
    );
    kassiber_envelope::set(
        &mut result,
        "daemon",
        s(kassiber_envelope::daemon_line(&envelope).unwrap()),
    );
    result
}
