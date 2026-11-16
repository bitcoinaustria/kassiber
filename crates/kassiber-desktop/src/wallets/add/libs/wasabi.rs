//! lib/wasabiBundle.ts: assemble the Wasabi RPC responses pasted into the Add connection form
//! into the `source_bundle` `ui.wallets.import_file` takes.

use serde_json::{Map, Value};

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Errors {
    pub history: Option<String>,
    pub coins: Option<String>,
    pub wallet_info: Option<String>,
    pub additional: Option<String>,
}

impl Errors {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.history.is_none()
            && self.coins.is_none()
            && self.wallet_info.is_none()
            && self.additional.is_none()
    }
}

pub struct Built {
    /// The bundle; the web always builds one, so this is `Some` (possibly partial) even when
    /// `errors` is not empty.
    pub bundle: Option<Value>,
    pub errors: Errors,
}

fn parse_field(value: &str, slot: &mut Option<String>) -> Option<Value> {
    let t = value.trim();
    if t.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(t) {
        Ok(v) => Some(v),
        Err(_) => {
            *slot = Some("Paste valid JSON from Wasabi RPC.".to_owned());
            None
        }
    }
}

/// `buildWasabiBundle`.
pub fn build_bundle(history: &str, coins: &str, wallet_info: &str, additional: &str) -> Built {
    let mut e = Errors::default();
    let mut bundle = Map::new();
    if let Some(extra) = parse_field(additional, &mut e.additional) {
        match extra {
            Value::Object(m) => bundle.extend(m),
            _ => {
                e.additional = Some("Additional Wasabi sections must be a JSON object.".to_owned())
            }
        }
    }
    match parse_field(history, &mut e.history) {
        None if e.history.is_none() => {
            e.history = Some("Paste the Wasabi gethistory JSON response.".to_owned())
        }
        None => {}
        Some(h) => {
            bundle.insert("gethistory".into(), h);
        }
    }
    if let Some(c) = parse_field(coins, &mut e.coins) {
        bundle.insert("listcoins".into(), c);
    }
    if let Some(w) = parse_field(wallet_info, &mut e.wallet_info) {
        bundle.insert("getwalletinfo".into(), w);
    }
    Built {
        bundle: Some(Value::Object(bundle)),
        errors: e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_bundle() {
        let b = build_bundle(
            r#"{"result":[{"tx":"aa"}]}"#,
            r#"{"result":[{"txid":"aa","index":0}]}"#,
            r#"{"result":{"walletName":"Wasabi"}}"#,
            r#"{"listpaymentsincoinjoin":{"result":[]}}"#,
        );
        assert!(b.errors.is_empty());
        assert_eq!(
            b.bundle.unwrap(),
            json!({
                "gethistory": {"result": [{"tx": "aa"}]},
                "listcoins": {"result": [{"txid": "aa", "index": 0}]},
                "getwalletinfo": {"result": {"walletName": "Wasabi"}},
                "listpaymentsincoinjoin": {"result": []},
            })
        );
    }

    #[test]
    fn requires_history() {
        let b = build_bundle("", "", "", "");
        assert!(b.errors.history.unwrap().contains("gethistory"));
    }

    #[test]
    fn rejects_invalid_json_and_non_object_additional() {
        assert!(
            build_bundle("{", "", "", "")
                .errors
                .history
                .unwrap()
                .contains("valid JSON")
        );
        assert!(
            build_bundle("[]", "", "", "[]")
                .errors
                .additional
                .unwrap()
                .contains("JSON object")
        );
    }
}
