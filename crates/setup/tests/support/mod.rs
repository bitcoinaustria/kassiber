#![allow(dead_code)]
use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::Value;
use kassiber_setup::{execute, Result, Sources};
use kassiber_store::{rusqlite::Connection, Book};
use std::cell::Cell;
pub struct FixedSources(pub Cell<u64>);
impl Sources for FixedSources {
    fn uuid(&self, _: &Connection) -> Result<String> {
        let n = self.0.get() + 1;
        self.0.set(n);
        Ok(format!("00000000-0000-4000-8000-{n:012x}"))
    }
    fn now(&self, _: &Connection) -> Result<String> {
        Ok("2026-01-01T00:00:00Z".into())
    }
}
pub struct Fixture {
    pub book: Book,
    pub sources: FixedSources,
    pub _root: tempfile::TempDir,
}
impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), None).unwrap();
        Self {
            book,
            sources: FixedSources(Cell::new(0)),
            _root: root,
        }
    }
    pub fn run(&self, op: &str, args: Value) -> Value {
        execute(self.book.connection(), op, &args, &self.sources).unwrap()
    }
    pub fn seed() -> Self {
        let f = Self::new();
        f.run("workspaces.create", dict([("label", s("Main"))]));
        f.run(
            "profiles.create",
            dict([
                ("label", s("Book")),
                ("fiat_currency", s("USD")),
                ("tax_country", s("generic")),
                ("tax_long_term_days", Value::Int(365.into())),
            ]),
        );
        f
    }
    pub fn fail(&self, op: &str, args: Value) -> kassiber_envelope::AppError {
        execute(self.book.connection(), op, &args, &self.sources).unwrap_err()
    }
    pub fn view(&self, surface: &str, name: &str) -> Value {
        self.run(
            "views.create",
            dict([
                ("surface", s(surface)),
                ("name", s(name)),
                ("filter_json", s(r#"{"foo":"bar"}"#)),
            ]),
        )
    }
    pub fn profile(&self) -> Value {
        self.run("profiles.get", dict::<&str>([]))
    }
    pub fn version(&self) -> i64 {
        self.book
            .connection()
            .query_row(
                "SELECT journal_input_version FROM profiles WHERE id=?",
                [text(&self.profile(), "id")],
                |r| r.get(0),
            )
            .unwrap()
    }
}
pub fn text(v: &Value, key: &str) -> String {
    py_str(get(v, key).unwrap())
}
