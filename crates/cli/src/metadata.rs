use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::metadata::{Bip329, Command, HistoryOptions, RecordOptions};

use crate::argparse::Namespace;

pub fn execute(book: &mut dyn Book, path: &str, ns: &Namespace) -> Result<Value, AppError> {
    let text = |key| ns.text(key).unwrap_or_default().to_owned();
    // Namespace::text intentionally treats an empty string as absent for
    // selectors. An authored empty note is a real value, distinct from null.
    let optional = |key| match ns.get(key) {
        Some(Value::Str(text)) => text.as_str().map(str::to_owned),
        _ => None,
    };
    let transaction = text("transaction");
    let reason = optional("reason");
    let limit = match ns.get("limit") {
        Some(Value::Int(n)) => Some(i64::try_from(n.clone()).unwrap_or(if *n > 0.into() {
            i64::MAX
        } else {
            i64::MIN
        })),
        _ => None,
    };
    let command = match path {
        "metadata.bip329.preview" => Command::Bip329(Bip329::Preview { file: text("file") }),
        "metadata.bip329.import" => Command::Bip329(Bip329::Import {
            file: text("file"),
            apply_ambiguous: ns.flag("apply_ambiguous"),
            source: "cli".into(),
        }),
        "metadata.bip329.list" => Command::Bip329(Bip329::List {
            cursor: optional("cursor"),
            limit,
        }),
        "metadata.bip329.export" => Command::Bip329(Bip329::Export {
            file: text("file"),
            wallets: optional("wallet")
                .filter(|s| !s.is_empty())
                .map(|s| vec![s]),
            mode: text("mode"),
        }),
        "metadata.tags.list" => Command::Tags,
        "metadata.tags.create" => Command::CreateTag {
            code: text("code"),
            label: text("label"),
        },
        "metadata.records.get" => Command::Get { transaction },
        "metadata.notes.set" | "metadata.records.note.set" => Command::Note {
            transaction,
            note: optional("note"),
            reason,
        },
        "metadata.notes.clear" | "metadata.records.note.clear" => Command::Note {
            transaction,
            note: None,
            reason,
        },
        "metadata.tags.add"
        | "metadata.records.tag.add"
        | "metadata.tags.remove"
        | "metadata.records.tag.remove" => Command::Tag {
            transaction,
            tag: text("tag"),
            remove: path.ends_with(".remove"),
            reason,
        },
        "metadata.exclude"
        | "metadata.records.excluded.set"
        | "metadata.include"
        | "metadata.records.excluded.clear" => Command::Excluded {
            transaction,
            excluded: path == "metadata.exclude" || path.ends_with(".set"),
            reason,
        },
        "metadata.records.kind.set" | "metadata.records.kind.clear" => Command::Kind {
            transaction,
            kind: optional("kind"),
            reason,
        },
        "metadata.records.history.stale" => Command::Stale,
        "metadata.records.list" => {
            if ns.flag("has_note") && ns.flag("no_note") {
                return Err(AppError::new(
                    "validation",
                    "--has-note and --no-note are mutually exclusive",
                ));
            }
            if ns.flag("excluded") && ns.flag("included") {
                return Err(AppError::new(
                    "validation",
                    "--excluded and --included are mutually exclusive",
                ));
            }
            Command::Records(RecordOptions {
                wallet: optional("wallet"),
                tag: optional("tag"),
                has_note: if ns.flag("has_note") {
                    Some(true)
                } else if ns.flag("no_note") {
                    Some(false)
                } else {
                    None
                },
                excluded: if ns.flag("excluded") {
                    Some(true)
                } else if ns.flag("included") {
                    Some(false)
                } else {
                    None
                },
                start: optional("start"),
                end: optional("end"),
                cursor: optional("cursor"),
                limit,
            })
        }
        "metadata.records.history.list" | "metadata.records.history.activity" => {
            Command::History(HistoryOptions {
                transaction: optional("transaction"),
                wallet: optional("wallet"),
                source: optional("source"),
                field_family: optional("field_family"),
                field: optional("field"),
                pricing_only: ns.flag("pricing_only"),
                ai_only: ns.flag("ai_only"),
                stale_only: ns.flag("stale_only"),
                start: optional("start"),
                end: optional("end"),
                cursor: optional("cursor"),
                limit,
                ..HistoryOptions::default()
            })
        }
        "metadata.records.history.revert" => Command::Revert {
            transaction,
            event: text("event"),
            field: optional("field"),
            source: "cli".into(),
            reason,
        },
        _ => unreachable!("registered metadata command"),
    };
    book.metadata(ns.get("workspace"), ns.get("profile"), &command)
}
