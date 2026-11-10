//! Test-only driver over captured snapshots and a caller-owned disposable book.
use super::{field, list, occurrences, AnalysisIndex, Text};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::Value as Sql, Connection};

/// The tables of the frozen Python schema the fixtures load. A fresh book
/// may have more; the dumps cover every table of the seeded one.
pub fn seed_table_count() -> usize {
    include_str!("../../../reports/tests/data/privacy_hygiene_seed.sql")
        .matches("CREATE TABLE ")
        .count()
}

pub fn tables(conn: &Connection) -> Vec<(String, Vec<Vec<Sql>>)> {
    let names = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(names.len(), seed_table_count());
    names
        .into_iter()
        .map(|name| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let count = stmt.column_count();
            let mut rows = stmt
                .query_map([], |r| {
                    (0..count)
                        .map(|i| r.get::<_, Sql>(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort_by_cached_key(|r| format!("{r:?}"));
            (name, rows)
        })
        .collect()
}

pub fn evaluate(case: &Value, conn: &Connection) -> Result<Value, AppError> {
    match field(case, "operation").as_str() {
        Some("mirror") => super::mirror::evaluate_fixture(case, conn),
        Some("handles") => super::handles::evaluate_fixture(case, conn),
        Some("views") => super::views::evaluate_fixture(case, conn),
        Some("sources") => {
            let result = super::sources::build_index_at(
                conn,
                field(case, "profile").as_str().unwrap_or("p"),
                field(case, "dataset_observer").as_str().unwrap_or("owner"),
                field(case, "as_of")
                    .as_str()
                    .unwrap_or("2026-01-01T00:00:00Z"),
            );
            match result {
                Ok(index) => {
                    let index =
                        index.observer(field(case, "observer").as_str().unwrap_or("owner"))?;
                    let result = index.snapshot();
                    if field(case, "summary") == &Value::Bool(true) {
                        let labels = list(field(result, "labels"));
                        Ok(dict([
                            ("snapshot_id", field(result, "snapshot_id").clone()),
                            ("coverage", field(result, "coverage").clone()),
                            ("label_count", Value::Int(labels.len().into())),
                            (
                                "first_label",
                                labels.first().cloned().unwrap_or(Value::Null),
                            ),
                            ("last_label", labels.last().cloned().unwrap_or(Value::Null)),
                        ]))
                    } else {
                        Ok(result.clone())
                    }
                }
                Err(error) => Ok(dict([("error", s(error.code))])),
            }
        }
        Some("ingestion") => {
            let mut builder = super::ingestion::ObservationBuilder::read(conn, "p")?;
            for request in list(field(case, "requests")) {
                let row = super::value::Value::from_py(field(request, "row"));
                match field(request, "source").as_str() {
                    Some("transaction") => {
                        builder.transaction(&row, field(request, "cached") == &Value::Bool(true))?
                    }
                    Some("inventory") => builder.inventory(&row)?,
                    Some("reference") => {
                        builder.reference(&row, field(request, "acquired") == &Value::Bool(true))?
                    }
                    Some("relations") => builder.relations(
                        row["decisions"].as_array().unwrap(),
                        row["economics"].as_array().unwrap(),
                        row["fresh"] == true,
                    )?,
                    _ => {
                        return Err(AppError::new(
                            "validation",
                            "Unknown ingestion fixture source",
                        ))
                    }
                }
            }
            Ok(builder
                .finish()?
                .observer(field(case, "observer").as_str().unwrap_or("owner"))?
                .snapshot()
                .clone())
        }
        Some("confirmations") => {
            let mut overlay = super::confirmations::ConfirmationOverlay::read(conn, "p")?;
            list(field(case, "nodes"))
                .iter()
                .map(|node| overlay.apply(node))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::List)
        }
        Some("observer") => {
            let mut index = AnalysisIndex::from_snapshot(field(case, "snapshot").clone())?;
            for observer in list(field(case, "observers")) {
                index = index.observer(observer.as_str().unwrap())?;
            }
            Ok(index.snapshot().clone())
        }
        Some("parse") => Ok(occurrences::parse_transaction_identity(
            field(case, "subject").as_str().unwrap(),
            field(case, "chain").as_str(),
            field(case, "network").as_str(),
        )
        .unwrap_or(Value::Null)),
        Some("occurrences") => {
            let mut resolver = occurrences::OccurrenceResolver::read(conn, "p")?;
            let mut result = Vec::new();
            for request in list(field(case, "requests")) {
                let row = field(request, "row");
                let chain = field(request, "chain").as_str().unwrap_or("bitcoin");
                let network = field(request, "network").as_str().unwrap_or("main");
                let source = resolver.source(row, chain, network)?;
                let id = resolver.transaction_id(
                    chain,
                    network,
                    field(request, "txid").as_str().unwrap_or(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    ),
                    &source,
                    field(request, "occurrence"),
                    field(request, "resolve") != &Value::Bool(false),
                )?;
                result.push(dict([
                    ("source", source.value()),
                    (
                        "accepted",
                        Value::Bool(
                            resolver.accepts(row, field(request, "unscoped") == &Value::Bool(true)),
                        ),
                    ),
                    ("id", s(id)),
                    (
                        "ambiguous",
                        Value::List(resolver.ambiguous().iter().map(|v| s(v.as_str())).collect()),
                    ),
                ]));
            }
            Ok(Value::List(result))
        }
        _ => Err(AppError::new(
            "validation",
            "Unknown analysis-index fixture operation",
        )),
    }
}
