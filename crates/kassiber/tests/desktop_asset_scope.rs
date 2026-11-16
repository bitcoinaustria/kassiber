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
