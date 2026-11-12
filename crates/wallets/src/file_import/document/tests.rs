use super::*;

fn dump(v: &Py) -> String {
    kassiber_pyjson::dumps(v, &Options::new().sort_keys(true)).unwrap()
}

#[test]
fn python_document_row_vectors() {
    let Py::List(cases) = kassiber_pytext::loads(include_str!(
        "../../../tests/vectors/python_document_rows.json"
    ))
    .unwrap() else {
        panic!()
    };
    for case in cases {
        let name = rendered(field(&case, "name"));
        let level = threshold(field(&case, "threshold"));
        let actual = level.and_then(|threshold| {
            draft_rows(
                field(&case, "payload"),
                &rendered(field(&case, "source_hash")),
                &threshold,
                "EUR",
                Some(&[1]),
            )
        });
        if let Some(expected) = get(&case, "error") {
            let error = actual.unwrap_err();
            let error = dict([
                ("code", s(error.code)),
                (
                    "message",
                    error
                        .message_codepoints
                        .map_or_else(|| s(error.message), |v| Py::Str(*v)),
                ),
                ("hint", error.hint.map_or(Py::Null, |v| s(v.as_ref()))),
                ("details", error.details.unwrap_or(Py::Null)),
                ("retryable", Py::Bool(error.retryable)),
            ]);
            assert_eq!(dump(&error), dump(expected), "{name}");
        } else {
            let actual = actual.unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(
                dump(&Py::List(actual.clone())),
                dump(field(&case, "rows")),
                "{name}"
            );
            let reviewed = actual
                .iter()
                .map(|row| {
                    reviewed_record(row, &rendered(field(&case, "source_hash")), "EUR")
                        .unwrap_or(Py::Null)
                })
                .collect();
            assert_eq!(
                dump(&Py::List(reviewed)),
                dump(field(&case, "reviewed")),
                "{name}: reviewed"
            );
            let threshold = threshold(field(&case, "threshold")).unwrap();
            let flags = actual
                .iter()
                .map(|row| {
                    Py::List(
                        confidence_flags(row, &threshold)
                            .into_iter()
                            .map(s)
                            .collect(),
                    )
                })
                .collect();
            assert_eq!(
                dump(&Py::List(flags)),
                dump(field(&case, "confidence_flags")),
                "{name}: confidence"
            );
        }
    }
}

fn fixture(encrypted: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    if encrypted {
        conn.execute_batch("PRAGMA key='disposable-test-key'")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let schema: Json =
        serde_json::from_str(include_str!("../../../../store/fixtures/creation.json")).unwrap();
    for sql in schema["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','ws','p','Source','custom','{}','2025-01-01');
        INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','p');
        UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=3,journal_input_version=7,fiat_currency='EUR';").unwrap();
    conn
}
fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn ready() -> Vec<Py> {
    let Py::List(cases) = kassiber_pytext::loads(include_str!(
        "../../../tests/vectors/python_document_rows.json"
    ))
    .unwrap() else {
        panic!()
    };
    let Py::List(rows) = field(&cases[0], "rows").clone() else {
        panic!()
    };
    rows
}
fn source() -> (files::Temp, std::path::PathBuf) {
    let root = files::Temp::new("document-test").unwrap();
    let source = root.path().join("receipt.png");
    std::fs::write(&source, b"Kassiber synthetic local OCR evidence\n").unwrap();
    (root, source)
}
fn import(
    conn: &Connection,
    root: &std::path::Path,
    source: &std::path::Path,
    rows: &[Py],
    attach: bool,
) -> Result<Py, AppError> {
    import_draft(
        conn,
        ImportDraft {
            source_file: source.to_str().unwrap(),
            data_root: root,
            profile_id: "p",
            wallet: "Source",
            rows,
            include_quarantined: false,
            selected_row_ids: None,
            expected_source_sha256: Some(
                "f85736148c94b4c6ae60695c6614f5c33945b8a80d1b50d255243b21ab57b7f5",
            ),
            confidence_threshold: &Py::Null,
            attach_evidence: attach,
            check_evidence_budget: false,
        },
        None,
    )
}
#[test]
fn document_coordinator_dedupes_attaches_only_changes_and_preserves_authority() {
    let (root, source) = source();
    for encrypted in [false, true] {
        let conn = fixture(encrypted);
        let mut rows = ready();
        let mut record = field(&rows[0], "record").clone();
        set(&mut record, "fiat_value", Py::Null);
        set(&mut record, "fiat_rate", Py::Null);
        set(&mut rows[0], "record", record);
        let outcome = import(&conn, root.path(), &source, &rows, true).unwrap();
        assert_eq!(field(&outcome, "imported"), &Py::from(1));
        assert_eq!(
            scalar(&conn, "SELECT amount FROM transactions"),
            1_000_000_000
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 1);
        assert_eq!(
            scalar(&conn, "SELECT journal_input_version FROM profiles"),
            8
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 0);
        for table in [
            "journal_custody_decisions",
            "custody_components",
            "journal_custody_gap_inputs",
            "transaction_pairs",
        ] {
            assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
        }
        let outcome = import(&conn, root.path(), &source, &rows, true).unwrap();
        assert_eq!(field(&outcome, "unchanged"), &Py::from(1));
        assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 1);
        assert_eq!(
            scalar(&conn, "SELECT journal_input_version FROM profiles"),
            8
        );
        // Equal-priority supplied prices count as updates in the reference,
        // including an identical priced reimport.
        for expected in [9, 10] {
            let outcome = import(&conn, root.path(), &source, &ready(), true).unwrap();
            assert_eq!(field(&outcome, "updated"), &Py::from(1));
            assert_eq!(
                scalar(&conn, "SELECT count(*) FROM attachments"),
                expected - 7
            );
            assert_eq!(
                scalar(&conn, "SELECT journal_input_version FROM profiles"),
                expected
            );
        }
    }
}
#[test]
fn document_late_attachment_failure_rolls_back_every_write_and_copy() {
    for encrypted in [false, true] {
        let (root, source) = source();
        let conn = fixture(encrypted);
        conn.execute_batch("CREATE TRIGGER late_document_failure BEFORE INSERT ON attachments WHEN (SELECT count(*) FROM attachments)>0 BEGIN SELECT RAISE(ABORT,'late evidence failure'); END;").unwrap();
        let mut rows = ready();
        let mut second = rows[0].clone();
        let id = rendered(field(&second, "id"));
        set(&mut second, "id", s(format!("{}002", &id[..id.len() - 3])));
        rows.push(second);
        let error = import(&conn, root.path(), &source, &rows, true).unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(error.message, "database operation failed");
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 0);
        assert_eq!(
            scalar(&conn, "SELECT journal_input_version FROM profiles"),
            7
        );
        assert_eq!(
            scalar(&conn, "SELECT last_processed_tx_count FROM profiles"),
            3
        );
        let files = std::fs::read_dir(root.path().join("attachments/p"))
            .unwrap()
            .count();
        assert_eq!(files, 0);
    }
}

#[test]
fn document_managed_digest_failure_rolls_back_and_removes_the_copy() {
    for encrypted in [false, true] {
        let (root, source) = source();
        let conn = fixture(encrypted);
        conn.execute_batch("CREATE TRIGGER corrupt_document_digest AFTER INSERT ON attachments BEGIN UPDATE attachments SET sha256='bad' WHERE id=NEW.id; END;").unwrap();
        let error = import(&conn, root.path(), &source, &ready(), true).unwrap_err();
        assert_eq!(error.code, "document_import_source_changed");
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 0);
        assert_eq!(
            scalar(&conn, "SELECT journal_input_version FROM profiles"),
            7
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("attachments/p"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn document_cleanup_never_follows_untrusted_stored_paths() {
    for encrypted in [false, true] {
        let (root, source) = source();
        let victim = root.path().join("victim.txt");
        std::fs::write(&victim, b"unrelated local file").unwrap();
        let conn = fixture(encrypted);
        conn.execute_batch("CREATE TRIGGER corrupt_document_path AFTER INSERT ON attachments BEGIN UPDATE attachments SET stored_relpath='../victim.txt' WHERE id=NEW.id; END;
            CREATE TRIGGER late_document_failure BEFORE INSERT ON attachments WHEN (SELECT count(*) FROM attachments)>0 BEGIN SELECT RAISE(ABORT,'late evidence failure'); END;").unwrap();
        let mut rows = ready();
        let mut second = rows[0].clone();
        let id = rendered(field(&second, "id"));
        set(&mut second, "id", s(format!("{}002", &id[..id.len() - 3])));
        rows.push(second);
        assert!(import(&conn, root.path(), &source, &rows, true).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"unrelated local file");
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 0);
        let managed_root = root.path().join("attachments");
        for relative in ["", "../victim.txt", victim.to_str().unwrap()] {
            assert!(files::managed_path(&managed_root, relative).is_none());
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path(), managed_root.join("escape")).unwrap();
            assert!(files::managed_path(&managed_root, "escape/victim.txt").is_none());
        }
    }
}

#[test]
fn document_attachment_context_refusals_roll_back_but_unattached_rows_import() {
    for encrypted in [false, true] {
        for missing in [true, false] {
            let (root, source) = source();
            let conn = fixture(encrypted);
            if missing {
                conn.execute("DELETE FROM settings WHERE key LIKE 'context_%'", [])
                    .unwrap();
            } else {
                conn.execute_batch("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('other','ws','Other','2025-01-01');
                    UPDATE settings SET value='other' WHERE key='context_profile';").unwrap();
            }
            let error = import(&conn, root.path(), &source, &ready(), true).unwrap_err();
            assert_eq!(error.code, "app_error");
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
            assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 0);
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT journal_input_version FROM profiles WHERE id='p'"
                ),
                7
            );
            let outcome = import(&conn, root.path(), &source, &ready(), false).unwrap();
            assert_eq!(field(&outcome, "imported"), &Py::from(1));
            assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 0);
        }
    }
}

#[test]
fn document_metadata_binding_refusals_preserve_codepoints_and_roll_back() {
    for encrypted in [false, true] {
        for key in ["counterparty", "description"] {
            let (root, source) = source();
            let conn = fixture(encrypted);
            let mut rows = ready();
            let mut record = field(&rows[0], "record").clone();
            set(
                &mut record,
                key,
                kassiber_pytext::loads(r#""Counter\ud800party""#).unwrap(),
            );
            set(&mut rows[0], "record", record);
            let error = import(&conn, root.path(), &source, &rows, true).unwrap_err();
            assert_eq!(error.code, "internal_error");
            assert_eq!(error.message, "'utf-8' codec can't encode character '\\ud800' in position 7: surrogates not allowed");
            assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
            assert_eq!(scalar(&conn, "SELECT count(*) FROM attachments"), 0);
            assert_eq!(
                scalar(&conn, "SELECT journal_input_version FROM profiles"),
                7
            );
        }
    }
}
#[test]
fn document_source_refusals_and_explicit_empty_selection_never_write() {
    let (root, source) = source();
    let conn = fixture(false);
    let rows = ready();
    let hash = files::hash(&source).unwrap();
    assert_eq!(
        hash,
        "f85736148c94b4c6ae60695c6614f5c33945b8a80d1b50d255243b21ab57b7f5"
    );
    let request = || ImportDraft {
        source_file: source.to_str().unwrap(),
        data_root: root.path(),
        profile_id: "p",
        wallet: "Source",
        rows: &rows,
        include_quarantined: true,
        selected_row_ids: Some(&[]),
        expected_source_sha256: Some(&hash),
        confidence_threshold: &Py::Null,
        attach_evidence: true,
        check_evidence_budget: false,
    };
    assert_eq!(
        import_draft(&conn, request(), None).unwrap_err().code,
        "document_import_no_ready_rows"
    );
    for expected in ["bad", &"0".repeat(64)] {
        let mut r = request();
        r.expected_source_sha256 = Some(expected);
        assert!(import_draft(&conn, r, None).is_err());
    }
    for url in [
        "https://drive.google.com/file/d/test",
        "http://127.0.0.1:1/a.pdf",
    ] {
        let mut r = request();
        r.source_file = url;
        assert_eq!(
            import_draft(&conn, r, None).unwrap_err().code,
            "document_import_url_not_supported"
        );
    }
    let unsupported = root.path().join("receipt.csv");
    std::fs::write(&unsupported, b"no OCR source").unwrap();
    let absent = root.path().join("absent.png");
    for (path, code) in [
        (root.path(), "validation"),
        (unsupported.as_path(), "validation"),
        (absent.as_path(), "not_found"),
    ] {
        let mut r = request();
        r.source_file = path.to_str().unwrap();
        assert_eq!(import_draft(&conn, r, None).unwrap_err().code, code);
    }
    std::fs::write(&source, b"changed").unwrap();
    let mut r = request();
    r.selected_row_ids = None;
    assert_eq!(
        import_draft(&conn, r, None).unwrap_err().code,
        "document_import_source_changed"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    assert_eq!(
        scalar(&conn, "SELECT journal_input_version FROM profiles"),
        7
    );
}

#[test]
fn document_snapshot_is_private_stable_and_removed_on_drop() {
    let (root, source) = source();
    let snapshot = files::Snapshot::new(&source).unwrap();
    let directory = snapshot.path.parent().unwrap().to_path_buf();
    assert_eq!(snapshot.path.file_name(), source.file_name());
    assert_eq!(
        std::fs::read(&snapshot.path).unwrap(),
        b"Kassiber synthetic local OCR evidence\n"
    );
    std::fs::write(&source, b"changed after snapshot").unwrap();
    assert_ne!(
        files::hash(&source).unwrap(),
        files::hash(&snapshot.path).unwrap()
    );
    assert!(!directory.starts_with(root.path()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    drop(snapshot);
    assert!(!directory.exists());
}

#[test]
fn python_document_review_vectors() {
    let Py::List(cases) = kassiber_pytext::loads(include_str!(
        "../../../tests/vectors/python_document_review.json"
    ))
    .unwrap() else {
        panic!()
    };
    for case in cases {
        let name = rendered(field(&case, "name"));
        let Py::List(rows) = field(&case, "rows") else {
            panic!()
        };
        let selected = match field(&case, "selected") {
            Py::List(v) => Some(v.iter().map(rendered).collect::<Vec<_>>()),
            _ => None,
        };
        let (records, skipped) = reviewed_records(
            rows,
            truthy(field(&case, "include_quarantined")),
            selected.as_deref(),
            &rendered(field(&case, "source_hash")),
            &rendered(field(&case, "currency")),
            field(&case, "threshold"),
        )
        .unwrap();
        assert_eq!(
            dump(&Py::List(records)),
            dump(field(&case, "records")),
            "{name}"
        );
        assert_eq!(&Py::from(skipped as i64), field(&case, "skipped"), "{name}");
    }
    assert_eq!(
        reviewed_records(
            &vec![ready()[0].clone(); 501],
            true,
            None,
            "source",
            "EUR",
            &Py::Null
        )
        .unwrap_err()
        .code,
        "validation"
    );
}

#[test]
fn document_evidence_budget_and_source_limit_refuse_before_writes() {
    let (root, source) = source();
    let conn = fixture(false);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    file.set_len(files::MAX_SOURCE + 1).unwrap();
    assert_eq!(
        files::source_path(source.to_str().unwrap())
            .unwrap_err()
            .code,
        "validation"
    );
    file.set_len(files::MAX_SOURCE).unwrap();
    drop(file);
    let hash = files::hash(&source).unwrap();
    let mut row = ready()[0].clone();
    let id = rendered(field(&row, "id"));
    set(
        &mut row,
        "id",
        s(id.replacen("f85736148c94b4c6", &hash[..16], 1)),
    );
    let rows = vec![row; 22];
    let request = ImportDraft {
        source_file: source.to_str().unwrap(),
        data_root: root.path(),
        profile_id: "p",
        wallet: "Source",
        rows: &rows,
        include_quarantined: false,
        selected_row_ids: None,
        expected_source_sha256: Some(&hash),
        confidence_threshold: &Py::Null,
        attach_evidence: true,
        check_evidence_budget: false,
    };
    assert_eq!(
        import_draft(&conn, request, None).unwrap_err().code,
        "document_import_evidence_budget_exceeded"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
    assert_eq!(
        scalar(&conn, "SELECT journal_input_version FROM profiles"),
        7
    );
    assert!(!root.path().join("attachments").exists());
}
