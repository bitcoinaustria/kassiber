//! The desktop's asset scope toggle (docs/plan/24-crypto-assets.md):
//! `ui.wallets.update` carries `asset_scope` behind its usual re-auth, the
//! wallet lifecycle stores it, rows already imported stay as they are, and
//! the exchange status names a scope only once a wallet stores one.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_pyjson::Value;
use support::{kind_of, list, path, text, Desk};

const UPDATE: &str = "ui.wallets.update";
const ACK: &str = r#""auth_response": {"plaintext_change_ack": "CHANGE LOCAL DATA"}"#;

fn update(desk: &Desk, fields: &str) -> Value {
    desk.ask(UPDATE, &format!("{{{fields}, {ACK}}}"))
}

fn stored_config(desk: &Desk, wallet: &str) -> serde_json::Value {
    let config: String = desk
        .connection()
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [wallet],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&config).unwrap()
}

fn transactions(desk: &Desk, wallet: &str) -> Vec<(String, String, i64)> {
    let conn = desk.connection();
    let mut stmt = conn
        .prepare("SELECT id,asset,amount FROM transactions WHERE wallet_id=? ORDER BY id")
        .unwrap();
    stmt.query_map([wallet], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn error_code(response: &Value) -> &str {
    assert_eq!(kind_of(response), "error", "{response:?}");
    path(response, &["error", "code"])
        .and_then(text)
        .unwrap_or_default()
}

fn status_scope(desk: &Desk) -> Option<String> {
    let status = desk.ask(
        "ui.connections.exchange.status",
        r#"{"wallet": "exchange"}"#,
    );
    assert_eq!(kind_of(&status), "ui.connections.exchange.status");
    path(
        &list(&status, &["data", "connections"])[0],
        &["asset_scope"],
    )
    .and_then(text)
    .map(str::to_owned)
}

#[test]
fn wallet_update_sets_the_scope_and_switching_back_keeps_imported_rows() {
    let desk = Desk::new("desktop_asset_scope_unrecorded");
    // The seed's Kraken wallet has no backend; only one with a key attached
    // is an exchange API connection the status answers for.
    desk.connection()
        .execute(
            "UPDATE wallets SET config_json='{\"backend\": \"kraken\"}' WHERE id='exchange'",
            [],
        )
        .unwrap();
    let rows = transactions(&desk, "exchange");
    assert!(!rows.is_empty());
    // A wallet that never stored a scope answers the status as before.
    assert_eq!(status_scope(&desk), None);

    let on = update(&desk, r#""wallet": "Exchange", "asset_scope": " Crypto ""#);
    assert_eq!(kind_of(&on), UPDATE, "{on:?}");
    assert_eq!(stored_config(&desk, "exchange")["asset_scope"], "crypto");
    assert_eq!(status_scope(&desk).as_deref(), Some("crypto"));

    // Switching back only stops future imports of other assets: the rows
    // already in the book stay exactly as they are.
    let off = update(&desk, r#""wallet": "exchange", "asset_scope": "bitcoin""#);
    assert_eq!(kind_of(&off), UPDATE, "{off:?}");
    assert_eq!(stored_config(&desk, "exchange")["asset_scope"], "bitcoin");
    assert_eq!(status_scope(&desk).as_deref(), Some("bitcoin"));
    assert_eq!(transactions(&desk, "exchange"), rows);
}

#[test]
fn wallet_update_refuses_a_bad_scope_and_a_change_without_re_auth() {
    let desk = Desk::new("desktop_asset_scope_unrecorded");
    for fields in [
        r#""wallet": "exchange", "asset_scope": "eth""#,
        r#""wallet": "exchange", "asset_scope": 1"#,
    ] {
        assert_eq!(error_code(&update(&desk, fields)), "validation", "{fields}");
    }
    // A plaintext book needs the typed acknowledgement, as for every other
    // wallet change.
    let unacknowledged = desk.ask(UPDATE, r#"{"wallet": "exchange", "asset_scope": "crypto"}"#);
    assert_eq!(error_code(&unacknowledged), "validation");
    let foreign = update(&desk, r#""wallet": "foreign", "asset_scope": "crypto""#);
    assert_eq!(error_code(&foreign), "not_found");
    assert!(stored_config(&desk, "exchange")
        .get("asset_scope")
        .is_none());
    assert!(stored_config(&desk, "foreign").get("asset_scope").is_none());
}

#[test]
fn an_encrypted_book_asks_for_the_passphrase_before_changing_the_scope() {
    let mut desk = Desk::new("desktop_asset_scope_unrecorded");
    desk.encrypt("synthetic scope passphrase");
    let asked = desk.ask(UPDATE, r#"{"wallet": "exchange", "asset_scope": "crypto"}"#);
    assert_eq!(kind_of(&asked), "auth_required", "{asked:?}");
    assert!(stored_config(&desk, "exchange")
        .get("asset_scope")
        .is_none());
    let done = desk.ask(
        UPDATE,
        r#"{"wallet": "exchange", "asset_scope": "crypto", "auth_response": {"passphrase_secret": "synthetic scope passphrase"}}"#,
    );
    assert_eq!(kind_of(&done), UPDATE, "{done:?}");
    assert_eq!(stored_config(&desk, "exchange")["asset_scope"], "crypto");
}

/// The asset question's preview binds the file it read
/// (docs/reference/imports.md#choosing-the-assets-before-the-first-import):
/// a create or import with its token refuses the file once it changed.
#[test]
fn a_preview_token_binds_the_create_and_import_to_the_previewed_file() {
    let desk = Desk::new("desktop_asset_scope_unrecorded");
    desk.change("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) SELECT 'treasury',workspace_id,id,'treasury','Treasury','asset','BTC','fixed' FROM profiles LIMIT 1");
    let file = desk.base_path().join("history.csv");
    let header = "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Trade-Group,Comment,Date,Tx-ID\n";
    std::fs::write(&file, format!("{header}Trade,0.1,BTC,3000,EUR,,,Kraken,,,2025-02-01,t-btc\nDeposit,3,ETH,,,,,Kraken,,,2025-02-03,d-eth\n")).unwrap();
    let file_path = file.to_string_lossy().into_owned();
    let preview = desk.ask(
        "ui.wallets.import_preview",
        &format!(r#"{{"source_file": "{file_path}", "source_format": "cointracking_csv", "label": "History"}}"#),
    );
    assert_eq!(
        kind_of(&preview),
        "ui.wallets.import_preview",
        "{preview:?}"
    );
    assert_eq!(path_text(&preview, &["data", "choice"]), "true");
    let token = path(&preview, &["data", "preview_token"])
        .and_then(text)
        .unwrap()
        .to_owned();
    let create = |label: &str| {
        desk.ask(
            "ui.wallets.create",
            &format!(r#"{{"label": "{label}", "kind": "cointracking", "source_file": "{file_path}", "source_format": "cointracking_csv", "asset_scope": "crypto", "preview_token": "{token}"}}"#),
        )
    };
    let created = create("History");
    assert_eq!(kind_of(&created), "ui.wallets.create", "{created:?}");
    let import = || {
        desk.ask(
            "ui.wallets.import_file",
            &format!(r#"{{"wallet": "History", "source_file": "{file_path}", "source_format": "cointracking_csv", "preview_token": "{token}"}}"#),
        )
    };
    // The file changes after the check: neither a create nor an import reads it.
    std::fs::write(&file, format!("{header}Trade,0.1,BTC,3000,EUR,,,Kraken,,,2025-02-01,t-btc\nDeposit,3,DOT,,,,,Kraken,,,2025-02-03,d-dot\n")).unwrap();
    assert_eq!(error_code(&import()), "import_preview_stale");
    assert_eq!(error_code(&create("History 2")), "import_preview_stale");
    let wallets: i64 = desk
        .connection()
        .query_row(
            "SELECT count(*) FROM wallets WHERE label='History 2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(wallets, 0);
}

fn path_text(value: &Value, keys: &[&str]) -> String {
    match path(value, keys) {
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => text(other).unwrap_or_default().to_owned(),
        None => String::new(),
    }
}
