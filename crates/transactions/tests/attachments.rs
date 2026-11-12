use kassiber_envelope::{get, s};
use kassiber_pyjson::Value;
use kassiber_store::Book;
use kassiber_transactions::{
    attachments::{execute, url_display_label, Command},
    Scope,
};
use std::fs;

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    match get(v, key).unwrap() {
        Value::Str(t) => t.as_str().unwrap(),
        _ => panic!(),
    }
}
fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}
fn book() -> (tempfile::TempDir, Book) {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(&tmp.path().join("data"), None).unwrap();
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','Book','2025-01-01T00:00:00Z'); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','w','p','Wallet','descriptor','2025-01-01T00:00:00Z'); INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES('tx','w','p','wal','fp','BTC','inbound',1001,0,'2025-01-01T00:00:00Z','deposit','2025-01-01T00:00:00Z'),('target','w','p','wal','fp2','BTC','inbound',1001,0,'2025-01-01T00:00:00Z','deposit','2025-01-01T00:00:00Z');").unwrap();
    (tmp, book)
}
fn add(
    book: &Book,
    root: &std::path::Path,
    file: Option<String>,
    url: Option<String>,
    label: Option<String>,
) -> Result<Value, kassiber_envelope::AppError> {
    execute(
        book.connection(),
        Some(&scope()),
        root,
        &Command::Add {
            transaction: "tx".into(),
            file,
            url,
            label,
            media_type: None,
        },
    )
}

#[test]
fn file_layout_name_digest_size_and_permissions_match_python() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let source = tmp.path().join(" .récéipt\t²\u{11f02}.txt ");
    fs::write(&source, b"invoice\n").unwrap();
    let row = add(
        &book,
        &root,
        Some(source.to_str().unwrap().into()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        text(&row, "stored_relpath"),
        format!("p/{}-récéipt_²_.txt", text(&row, "id"))
    );
    assert_eq!(get(&row, "size_bytes"), Some(&Value::Int(8.into())));
    assert_eq!(
        text(&row, "sha256"),
        "3dc92b8f81254df44a02a330fa4c565c53cfe681a17cc42cb5bda34d7f6de5a4"
    );
    let managed = root.join(text(&row, "stored_relpath"));
    assert_eq!(fs::read(&managed).unwrap(), b"invoice\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Compare files/dirs created under the same process umask.
        let reference = tmp.path().join("reference");
        fs::write(&reference, b"").unwrap();
        let reference_dir = tmp.path().join("reference-dir");
        fs::create_dir(&reference_dir).unwrap();
        assert_eq!(
            managed.metadata().unwrap().permissions().mode(),
            reference.metadata().unwrap().permissions().mode()
        );
        assert_eq!(
            root.join("p").metadata().unwrap().permissions().mode(),
            reference_dir.metadata().unwrap().permissions().mode()
        );
    }
}

#[test]
fn literal_urls_are_local_references_and_only_their_label_can_be_renamed() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let url = "https://127.0.0.1:1/invoice.pdf?private=literal";
    let row = add(&book, &root, None, Some(url.into()), None).unwrap();
    assert_eq!(text(&row, "url"), url);
    assert_eq!(get(&row, "exists"), Some(&Value::Null));
    assert_eq!(text(&row, "stored_relpath"), "");
    assert_eq!(
        url_display_label("https://docs.google.com/spreadsheets/d/1/edit"),
        "Google Sheet"
    );
    assert_eq!(
        url_display_label("https://www.example.test/my%20invoice.PDF"),
        "example.test - my invoice"
    );
    assert_eq!(
        url_display_label("custom://local/reference"),
        "Link attachment"
    );
    assert_eq!(
        url_display_label("https://example.test/%€"),
        "example.test - %€"
    );
    let updated = execute(
        book.connection(),
        Some(&scope()),
        &root,
        &Command::Rename {
            attachment: text(&row, "id").into(),
            label: "  Board\n decision  ".into(),
        },
    )
    .unwrap();
    assert_eq!(text(&updated, "label"), "Board decision");
    assert_eq!(text(&updated, "url"), url);
}

#[test]
fn invalid_label_and_invalid_source_copy_nothing() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let source = tmp.path().join("receipt.txt");
    fs::write(&source, b"invoice").unwrap();
    for url in [
        "https://[broken",
        "https://[localhost]/",
        "https://[127.0.0.1]/",
    ] {
        assert_eq!(
            add(&book, &root, None, Some(url.into()), None)
                .unwrap_err()
                .code,
            "validation"
        );
    }
    for delimiter in "⁇⁈⁉℀℁℅℆⩴︓︖﹕﹖﹟﹫＃／：？＠".chars() {
        assert_eq!(
            add(
                &book,
                &root,
                None,
                Some(format!("https://example{delimiter}test/receipt")),
                None
            )
            .unwrap_err()
            .code,
            "validation"
        );
    }
    assert_eq!(
        book.connection()
            .query_row("SELECT COUNT(*) FROM attachments", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    for label in [" ".to_owned(), "x".repeat(201)] {
        assert_eq!(
            add(
                &book,
                &root,
                Some(source.to_str().unwrap().into()),
                None,
                Some(label)
            )
            .unwrap_err()
            .code,
            "validation"
        );
    }
    assert!(!root.join("p").exists());
    assert_eq!(
        add(&book, &root, None, Some("relative/ref".into()), None)
            .unwrap_err()
            .code,
        "validation"
    );
    assert_eq!(
        add(&book, &root, Some("/does/not/exist".into()), None, None)
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn copy_creates_independent_file_identity_and_preserves_provenance() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let source = tmp.path().join("receipt.txt");
    fs::write(&source, b"invoice").unwrap();
    let original = add(
        &book,
        &root,
        Some(source.to_str().unwrap().into()),
        None,
        None,
    )
    .unwrap();
    let url = add(
        &book,
        &root,
        None,
        Some("https://127.0.0.1:1/".into()),
        None,
    )
    .unwrap();
    let copied = execute(
        book.connection(),
        Some(&scope()),
        &root,
        &Command::Copy {
            transaction: "target".into(),
            source_transaction: "tx".into(),
            attachments: vec![
                s(text(&original, "id")),
                s(text(&url, "id")),
                s(text(&original, "id")),
            ],
        },
    )
    .unwrap();
    assert_eq!(get(&copied, "copied"), Some(&Value::Int(2.into())));
    let Value::List(copies) = get(&copied, "attachments").unwrap() else {
        panic!()
    };
    let copy = &copies[0];
    assert_ne!(text(copy, "id"), text(&original, "id"));
    assert_ne!(
        text(copy, "stored_relpath"),
        text(&original, "stored_relpath")
    );
    assert_eq!(
        text(copy, "copied_from_attachment_id"),
        text(&original, "id")
    );
    assert_eq!(text(copy, "copied_from_transaction_id"), "tx");
    execute(
        book.connection(),
        Some(&scope()),
        &root,
        &Command::Remove {
            attachment: text(&original, "id").into(),
        },
    )
    .unwrap();
    assert_eq!(
        fs::read(root.join(text(copy, "stored_relpath"))).unwrap(),
        b"invoice"
    );
    assert_eq!(text(&copies[1], "url"), text(&url, "url"));
}

#[test]
fn corrupt_copy_rolls_back_prior_rows_and_files() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let source = tmp.path().join("receipt.txt");
    fs::write(&source, b"invoice").unwrap();
    let original = add(
        &book,
        &root,
        Some(source.to_str().unwrap().into()),
        None,
        None,
    )
    .unwrap();
    let corrupt = add(
        &book,
        &root,
        Some(source.to_str().unwrap().into()),
        None,
        None,
    )
    .unwrap();
    fs::write(root.join(text(&corrupt, "stored_relpath")), b"damaged").unwrap();
    let result = execute(
        book.connection(),
        Some(&scope()),
        &root,
        &Command::Copy {
            transaction: "target".into(),
            source_transaction: "tx".into(),
            attachments: vec![s(text(&original, "id")), s(text(&corrupt, "id"))],
        },
    );
    assert_eq!(result.unwrap_err().code, "validation");
    assert_eq!(fs::read_dir(root.join("p")).unwrap().count(), 2);
    assert_eq!(
        book.connection()
            .query_row("SELECT COUNT(*) FROM attachments", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn traversal_verify_remove_and_open_never_touch_external_file() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let source = tmp.path().join("outside.txt");
    fs::write(&source, b"unrelated").unwrap();
    for relative in [
        "../outside.txt".to_string(),
        source.to_str().unwrap().into(),
    ] {
        let row = add(
            &book,
            &root,
            Some(source.to_str().unwrap().into()),
            None,
            None,
        )
        .unwrap();
        let id = text(&row, "id");
        book.connection()
            .execute(
                "UPDATE attachments SET stored_relpath=? WHERE id=?",
                [relative.as_str(), id],
            )
            .unwrap();
        let checked = execute(
            book.connection(),
            Some(&scope()),
            &root,
            &Command::Verify { transaction: None },
        )
        .unwrap();
        assert_eq!(get(&checked, "broken"), Some(&Value::Int(1.into())));
        assert_eq!(
            execute(
                book.connection(),
                Some(&scope()),
                &root,
                &Command::Open {
                    attachment: id.into()
                }
            )
            .unwrap_err()
            .code,
            "validation"
        );
        let removed = execute(
            book.connection(),
            Some(&scope()),
            &root,
            &Command::Remove {
                attachment: id.into(),
            },
        )
        .unwrap();
        assert_eq!(get(&removed, "deleted_file"), Some(&Value::Bool(false)));
        assert_eq!(fs::read(&source).unwrap(), b"unrelated");
    }
}

#[test]
fn failed_file_deletion_retains_row() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let row = add(
        &book,
        &root,
        None,
        Some("https://example.test".into()),
        None,
    )
    .unwrap();
    fs::create_dir(root.join("directory")).unwrap();
    book.connection()
        .execute(
            "UPDATE attachments SET stored_relpath='directory' WHERE id=?",
            [text(&row, "id")],
        )
        .unwrap();
    let result = execute(
        book.connection(),
        Some(&scope()),
        &root,
        &Command::Remove {
            attachment: text(&row, "id").into(),
        },
    );
    assert_eq!(result.unwrap_err().code, "filesystem_error");
    assert_eq!(
        book.connection()
            .query_row("SELECT COUNT(*) FROM attachments", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn gc_preserves_live_files_and_dry_run_changes_nothing() {
    let (tmp, book) = book();
    let root = tmp.path().join("attachments");
    let source = tmp.path().join("receipt.txt");
    fs::write(&source, b"invoice").unwrap();
    let row = add(
        &book,
        &root,
        Some(source.to_str().unwrap().into()),
        None,
        None,
    )
    .unwrap();
    fs::create_dir_all(root.join("orphan")).unwrap();
    fs::write(root.join("orphan/lost.bin"), b"orphan").unwrap();
    let dry = execute(
        book.connection(),
        None,
        &root,
        &Command::Gc { dry_run: true },
    )
    .unwrap();
    assert_eq!(get(&dry, "orphaned_bytes"), Some(&Value::Int(6.into())));
    assert!(root.join("orphan/lost.bin").exists());
    let result = execute(
        book.connection(),
        None,
        &root,
        &Command::Gc { dry_run: false },
    )
    .unwrap();
    assert_eq!(get(&result, "removed_files"), Some(&Value::Int(1.into())));
    assert!(!root.join("orphan").exists());
    assert!(root.join(text(&row, "stored_relpath")).exists());
}
