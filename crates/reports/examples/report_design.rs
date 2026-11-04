//! Render the summary report in the shared design for one book:
//! `report_design <database> <out-dir>` writes `model.json` (the data handed
//! to the renderers), `summary.pdf`, `summary.xlsx` and `summary-csv/`.
//! The clock is fixed so two runs must write identical bytes.
use kassiber_envelope::{dict, s};
use kassiber_store::rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        unreachable!("rendering mints no identity")
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let conn =
        Connection::open_with_flags(&args[1], OpenFlags::SQLITE_OPEN_READ_ONLY).expect("book");
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(out.join("summary-csv")).unwrap();
    let request = dict([
        ("start", s("2024-01-01T00:00:00Z")),
        ("end", s("2025-12-31T23:59:59Z")),
        ("include_snapshot", kassiber_pyjson::Value::Bool(true)),
    ]);
    let model =
        kassiber_reports::report_design::summary_model(&conn, &request, &Fixed).expect("model");
    let json =
        kassiber_pyjson::dumps(&model, &kassiber_pyjson::Options::new().sort_keys(true)).unwrap();
    std::fs::write(out.join("model.json"), json).unwrap();
    let document = kassiber_reports::report_design::summary(&model);
    let (pdf, pages) = kassiber_reports::report_design::pdf(&document).expect("pdf");
    std::fs::write(out.join("summary.pdf"), pdf).unwrap();
    std::fs::write(
        out.join("summary.xlsx"),
        kassiber_reports::report_design::xlsx(&document).expect("xlsx"),
    )
    .unwrap();
    for (name, bytes) in kassiber_reports::report_design::csv(&document).expect("csv") {
        std::fs::write(out.join("summary-csv").join(name), bytes).unwrap();
    }
    println!("{pages} pages");
}
