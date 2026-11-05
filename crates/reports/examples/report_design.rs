//! Render every report rolled out in the shared design for one book:
//! `report_design <database> <out-dir>` writes, per report, `<report>/`
//! with `model.json` (the data handed to the renderers), `<report>.pdf`,
//! `<report>.xlsx` and `<report>-csv/`. The clock is fixed so two runs must
//! write identical bytes.
use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_reports::report_design::{self, Document};
use kassiber_store::rusqlite::{Connection, OpenFlags};
use std::path::Path;

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        unreachable!("rendering mints no identity")
    }
}

fn write(out: &Path, name: &str, model: &Value, document: &Document) {
    let dir = out.join(name);
    std::fs::create_dir_all(dir.join(format!("{name}-csv"))).unwrap();
    let json =
        kassiber_pyjson::dumps(model, &kassiber_pyjson::Options::new().sort_keys(true)).unwrap();
    std::fs::write(dir.join("model.json"), json).unwrap();
    let (pdf, pages) = report_design::pdf(document).expect("pdf");
    std::fs::write(dir.join(format!("{name}.pdf")), pdf).unwrap();
    std::fs::write(
        dir.join(format!("{name}.xlsx")),
        report_design::xlsx(document).expect("xlsx"),
    )
    .unwrap();
    for (file, bytes) in report_design::csv(document).expect("csv") {
        std::fs::write(dir.join(format!("{name}-csv")).join(file), bytes).unwrap();
    }
    println!("{name}: {pages} pages");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let conn =
        Connection::open_with_flags(&args[1], OpenFlags::SQLITE_OPEN_READ_ONLY).expect("book");
    let out = Path::new(&args[2]);
    let request = dict([
        ("start", s("2024-01-01T00:00:00Z")),
        ("end", s("2025-12-31T23:59:59Z")),
        ("include_snapshot", Value::Bool(true)),
    ]);
    let model = report_design::summary_model(&conn, &request, &Fixed).expect("summary");
    write(out, "summary", &model, &report_design::summary(&model));

    let request = dict::<&str>([]);
    let (workspace, profile) = kassiber_setup::references::scope(&conn, &request).expect("book");
    let model = kassiber_reports::full_pdf::model(&conn, &workspace, &profile, &request, &Fixed)
        .expect("full");
    write(out, "full", &model, &report_design::full(&model));

    let request = dict([
        ("departure_date", s("2025-12-13")),
        ("destination", s("eu_eea")),
    ]);
    let model = kassiber_reports::exit_pdf::model(&conn, &request, &Fixed).expect("exit tax");
    write(out, "exit_tax", &model, &report_design::exit_tax(&model));

    if kassiber_envelope::get(&profile, "tax_country") == Some(&s("at")) {
        let request = dict([("year", Value::Int(2025.into()))]);
        let model =
            kassiber_reports::austrian_pdf::model(&conn, &workspace, &profile, &request, &Fixed)
                .expect("austrian");
        write(out, "austrian", &model, &report_design::austrian(&model));
    }
}
