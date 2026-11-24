//! The supervisor's AI-key bridge (supervisor.rs `augment_ai_provider_secret_args`,
//! `handle_secret_store_control_request`, lib.rs `attach_secret_store_policy_status`).
//!
//! - Requests of the AI provider kinds go out with `_desktop_secret_store_bridge: true`, so the
//!   daemon may keep keys in the OS store and ask the desktop for them; `set_api_key` and
//!   `move_api_key` also carry the policy's default store (`_desktop_secret_store_default`,
//!   `_desktop_secret_store_policy`). A key typed in Settings therefore lands where the Tauri
//!   app puts it: SQLCipher in unsigned/ad-hoc macOS builds unless the user moves it, the
//!   Keychain in production-signed ones, the user's Credential Manager / Secret Service on
//!   Windows / Linux.
//! - The daemon's `supervisor.ai_secret_store.request` lines are answered on the reader thread
//!   (`availability`, `get`, `exists`, `set`, `delete`), confined to the project's namespace (a
//!   64-hex `service`, `account` = the provider name) and the platform's own store id.
//!   SQLCipher refs never reach this side. The answer goes back on stdin and nowhere else: the
//!   secret is never logged, and its buffers are cleared once written.
//! - `ai.providers.list` answers gain `secret_store_policy` for the Settings AI section.

use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::store::{
    NativeSecretStore, STORE_ID_SQLCIPHER_INLINE, SecretStore,
    current_ai_provider_secret_store_policy, current_secret_store_platform,
    native_store_id_for_platform, secret_store_policy_status,
};

pub const SECRET_STORE_CONTROL_REQUEST_KIND: &str = "supervisor.ai_secret_store.request";
pub const SECRET_STORE_CONTROL_RESPONSE_KIND: &str = "supervisor.ai_secret_store.response";

static STORE: OnceLock<Mutex<Arc<dyn SecretStore>>> = OnceLock::new();

fn slot() -> &'static Mutex<Arc<dyn SecretStore>> {
    STORE.get_or_init(|| Mutex::new(Arc::new(NativeSecretStore)))
}

/// The store the bridge answers from (the OS store; tests swap in a memory one).
pub fn store() -> Arc<dyn SecretStore> {
    slot().lock().expect("bridge store").clone()
}

#[allow(dead_code)] // tests
pub fn set_store(store: Arc<dyn SecretStore>) {
    *slot().lock().expect("bridge store") = store;
}

pub fn uses_ai_provider_secret_bridge(kind: &str) -> bool {
    matches!(
        kind,
        "ai.providers.list"
            | "ai.providers.get"
            | "ai.providers.set_api_key"
            | "ai.providers.move_api_key"
            | "ai.providers.delete"
            | "ai.list_models"
            | "ai.test_connection"
            | "ai.chat"
    )
}

/// `augment_ai_provider_secret_args`: the request's args as they go to the daemon.
pub fn augment_args(kind: &str, args: &Value) -> Option<Value> {
    if !uses_ai_provider_secret_bridge(kind) {
        return None;
    }
    let mut map = match args {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        other => Map::from_iter([("_desktop_original_args".to_string(), other.clone())]),
    };
    map.insert(
        "_desktop_secret_store_bridge".to_string(),
        Value::Bool(true),
    );
    if matches!(
        kind,
        "ai.providers.set_api_key" | "ai.providers.move_api_key"
    ) {
        let requested = map
            .get("store_id")
            .and_then(Value::as_str)
            .filter(|v| *v != STORE_ID_SQLCIPHER_INLINE);
        let selection = current_ai_provider_secret_store_policy(requested);
        map.insert(
            "_desktop_secret_store_default".to_string(),
            Value::String(selection.store_id.clone()),
        );
        map.insert(
            "_desktop_secret_store_policy".to_string(),
            serde_json::to_value(selection).unwrap_or(Value::Null),
        );
    }
    Some(Value::Object(map))
}

/// `attach_secret_store_policy_status`.
pub fn attach_policy_status(envelope: &mut Value) {
    if envelope.get("kind").and_then(Value::as_str) != Some("ai.providers.list") {
        return;
    }
    if let Some(data) = envelope.get_mut("data").and_then(Value::as_object_mut) {
        data.insert(
            "secret_store_policy".to_string(),
            secret_store_policy_status(),
        );
    }
}

/// The JSON line answering one control request. Zeroized on drop: a `get` answer holds the key.
pub fn control_response_line(request: &Value, store: &dyn SecretStore) -> Zeroizing<String> {
    let request_id = request.get("request_id").cloned().unwrap_or(Value::Null);
    let response = match handle_operation(request, store) {
        Ok(data) => {
            json!({"kind": SECRET_STORE_CONTROL_RESPONSE_KIND, "schema_version": 1, "request_id": request_id, "data": data})
        }
        Err(message) => json!({
            "kind": SECRET_STORE_CONTROL_RESPONSE_KIND,
            "schema_version": 1,
            "request_id": request_id,
            "error": {"code": "secret_store_bridge_error", "message": crate::daemon::redact(&message), "retryable": true},
        }),
    };
    let line = Zeroizing::new(response.to_string());
    scrub(response);
    line
}

/// Clears the strings of a JSON value before it is freed (the `secret` field of a `get`).
pub fn scrub(value: Value) {
    match value {
        Value::String(s) => drop(Zeroizing::new(s)),
        Value::Array(a) => a.into_iter().for_each(scrub),
        Value::Object(m) => m.into_iter().for_each(|(_, v)| scrub(v)),
        _ => {}
    }
}

fn handle_operation(request: &Value, store: &dyn SecretStore) -> Result<Value, String> {
    let data = request
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "secret-store bridge request missing data".to_string())?;
    let provider_name = data
        .get("provider_name")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| "secret-store bridge request missing provider_name".to_string())?;
    let op = data
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| "secret-store bridge request missing op".to_string())?;
    let store_id = data
        .get("store_id")
        .and_then(Value::as_str)
        .unwrap_or(STORE_ID_SQLCIPHER_INLINE);
    if store_id == STORE_ID_SQLCIPHER_INLINE {
        return Err("sqlcipher_inline refs are handled by the daemon".to_string());
    }
    let Some(native_store_id) = native_store_id_for_platform(&current_secret_store_platform())
    else {
        return Err("this platform does not support native AI provider secret storage".to_string());
    };
    if store_id != native_store_id {
        return Err(format!(
            "requested AI provider secret store {store_id:?} is not available on this platform"
        ));
    }
    match op {
        "availability" => {
            Ok(json!({"provider_name": provider_name, "availability": store.availability()}))
        }
        "get" => {
            let (service, account) = secret_ref_service_account(data)?;
            match store.get(service, account)? {
                Some(mut bytes) => {
                    let text = std::str::from_utf8(&bytes).map(str::to_owned);
                    super::store::zero(&mut bytes);
                    let secret =
                        text.map_err(|_| "stored provider API key is not UTF-8".to_string())?;
                    Ok(json!({"provider_name": provider_name, "state": "ok", "secret": secret}))
                }
                None => Ok(
                    json!({"provider_name": provider_name, "state": "missing", "secret": Value::Null}),
                ),
            }
        }
        "exists" => {
            let (service, account) = secret_ref_service_account(data)?;
            let state = if store.exists(service, account)? {
                "ok"
            } else {
                "missing"
            };
            Ok(json!({"provider_name": provider_name, "state": state}))
        }
        "set" => {
            let (service, account) = secret_ref_service_account(data)?;
            let secret = data
                .get("secret")
                .and_then(Value::as_str)
                .ok_or_else(|| "secret-store set request missing secret".to_string())?;
            store.set(service, account, secret.as_bytes())?;
            Ok(json!({"provider_name": provider_name, "state": "ok"}))
        }
        "delete" => {
            let (service, account) = secret_ref_service_account(data)?;
            store.delete(service, account)?;
            Ok(json!({"provider_name": provider_name, "state": "missing"}))
        }
        other => Err(format!("unsupported secret-store bridge op {other:?}")),
    }
}

fn secret_ref_service_account(data: &Map<String, Value>) -> Result<(&str, &str), String> {
    let nonempty = |k: &str| {
        data.get(k)
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
    };
    let provider_name = nonempty("provider_name")
        .ok_or_else(|| "secret-store bridge request missing provider_name".to_string())?;
    let service = nonempty("service")
        .ok_or_else(|| "secret-store bridge request missing service".to_string())?;
    let account = nonempty("account")
        .ok_or_else(|| "secret-store bridge request missing account".to_string())?;
    if account != provider_name {
        return Err("secret-store account must match the AI provider name".to_string());
    }
    if service.len() != 64 || !service.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("secret-store service is outside the Kassiber namespace".to_string());
    }
    Ok((service, account))
}

#[cfg(test)]
mod tests {
    use super::super::store::{
        IdentityStrength, MemoryStore, SecretStoreAvailability, native_store_id_for_platform,
    };
    use super::*;

    const SERVICE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn mem() -> MemoryStore {
        MemoryStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        })
    }

    fn native_id() -> &'static str {
        native_store_id_for_platform(&current_secret_store_platform()).expect("a desktop platform")
    }

    fn req(op: &str, extra: Value) -> Value {
        let mut data = json!({"op": op, "provider_name": "remote", "store_id": native_id(), "service": SERVICE, "account": "remote"});
        for (k, v) in extra.as_object().unwrap() {
            data[k] = v.clone();
        }
        json!({"kind": SECRET_STORE_CONTROL_REQUEST_KIND, "schema_version": 1, "request_id": "secret-control-1", "data": data})
    }

    fn answer(request: &Value, store: &dyn SecretStore) -> Value {
        serde_json::from_str(&control_response_line(request, store)).unwrap()
    }

    #[test]
    fn set_get_exists_delete_roundtrip_through_the_bridge() {
        let s = mem();
        let r = answer(&req("set", json!({"secret": "sk-test"})), &s);
        assert_eq!(r["kind"], SECRET_STORE_CONTROL_RESPONSE_KIND);
        assert_eq!(r["request_id"], "secret-control-1");
        assert_eq!(r["data"]["state"], "ok");
        assert_eq!(answer(&req("exists", json!({})), &s)["data"]["state"], "ok");
        let got = answer(&req("get", json!({})), &s);
        assert_eq!(got["data"]["secret"], "sk-test");
        assert_eq!(got["data"]["provider_name"], "remote");
        assert_eq!(
            answer(&req("delete", json!({})), &s)["data"]["state"],
            "missing"
        );
        let gone = answer(&req("get", json!({})), &s);
        assert_eq!(gone["data"]["state"], "missing");
        assert!(gone["data"]["secret"].is_null());
        assert_eq!(
            answer(&req("availability", json!({})), &s)["data"]["availability"]["state"],
            "available"
        );
    }

    #[test]
    fn requests_outside_the_namespace_are_refused() {
        let s = mem();
        for (bad, why) in [
            (
                json!({"service": "not-hex"}),
                "outside the Kassiber namespace",
            ),
            (
                json!({"account": "other"}),
                "must match the AI provider name",
            ),
            (
                json!({"store_id": "sqlcipher_inline"}),
                "handled by the daemon",
            ),
            (
                json!({"store_id": "some_other_store"}),
                "not available on this platform",
            ),
            (json!({"provider_name": " "}), "missing provider_name"),
        ] {
            let r = answer(&req("get", bad), &s);
            assert_eq!(r["error"]["code"], "secret_store_bridge_error");
            assert_eq!(r["error"]["retryable"], true);
            assert!(r["error"]["message"].as_str().unwrap().contains(why), "{r}");
        }
        assert!(
            answer(&req("wipe", json!({})), &s)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("unsupported secret-store bridge op")
        );
    }

    #[test]
    fn store_errors_are_answered_redacted_and_retryable() {
        let s = mem();
        s.fail_next_set("denied by ACL");
        let r = answer(&req("set", json!({"secret": "sk-test"})), &s);
        assert_eq!(r["error"]["message"], "denied by ACL");
        assert!(!r.to_string().contains("sk-test"));
    }

    #[test]
    fn only_ai_provider_kinds_are_augmented() {
        assert_eq!(augment_args("ui.overview.snapshot", &Value::Null), None);
        let list = augment_args("ai.providers.list", &Value::Null).unwrap();
        assert_eq!(list, json!({"_desktop_secret_store_bridge": true}));
        let chat = augment_args("ai.chat", &json!({"prompt": "hi"})).unwrap();
        assert_eq!(chat["prompt"], "hi");
        assert_eq!(chat["_desktop_secret_store_bridge"], true);
        let odd = augment_args("ai.providers.get", &json!(["x"])).unwrap();
        assert_eq!(odd["_desktop_original_args"], json!(["x"]));
    }

    #[test]
    fn key_writes_carry_the_store_policy() {
        let set = augment_args(
            "ai.providers.set_api_key",
            &json!({"name": "remote", "api_key": "sk"}),
        )
        .unwrap();
        let default = current_ai_provider_secret_store_policy(None);
        assert_eq!(
            set["_desktop_secret_store_default"],
            json!(default.store_id)
        );
        assert_eq!(
            set["_desktop_secret_store_policy"]["reason"],
            serde_json::to_value(&default.reason).unwrap()
        );
        // An explicit SQLCipher move is not a native request.
        let mv = augment_args(
            "ai.providers.move_api_key",
            &json!({"name": "remote", "store_id": "sqlcipher_inline"}),
        )
        .unwrap();
        assert_eq!(mv["_desktop_secret_store_default"], json!(default.store_id));
    }

    /// End to end through the supervisor: a stub daemon asks for a key on the bridge while it
    /// answers `ai.providers.get`, reads the supervisor's answer from stdin and reports what it
    /// saw. The store is in memory: no OS Keychain is touched.
    #[cfg(unix)]
    #[test]
    fn the_supervisor_answers_the_daemons_bridge_requests() {
        use crate::daemon::{Incoming, Launch, Supervisor};
        use std::time::{Duration, Instant};

        let s = mem();
        s.set(SERVICE, "remote", b"sk-bridge-test").unwrap();
        set_store(std::sync::Arc::new(s));
        let dir = std::env::temp_dir().join(format!("rune-secrets-bridge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kassiber-bridge-stub.sh");
        let script = r#"#!/bin/sh
echo '{"kind":"daemon.ready","schema_version":1,"data":{}}'
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  kind=$(printf '%s' "$line" | sed -n 's/.*"kind":"\([^"]*\)".*/\1/p')
  case "$line" in *_desktop_secret_store_bridge*) bridged=true ;; *) bridged=false ;; esac
  case "$kind" in
    ai.providers.get)
      echo '{"kind":"supervisor.ai_secret_store.request","schema_version":1,"request_id":"ctl-1","data":{"op":"get","provider_name":"remote","store_id":"__STORE__","service":"__SERVICE__","account":"remote"}}'
      IFS= read -r answer
      state=$(printf '%s' "$answer" | sed -n 's/.*"state":"\([^"]*\)".*/\1/p')
      case "$answer" in *sk-bridge-test*) seen=yes ;; *) seen=no ;; esac
      case "$answer" in *secret-control-response*|*'"request_id":"ctl-1"'*) echoed=yes ;; *) echoed=no ;; esac
      echo "{\"kind\":\"ai.providers.get\",\"schema_version\":1,\"request_id\":\"$id\",\"data\":{\"bridged\":$bridged,\"state\":\"$state\",\"secret_seen\":\"$seen\",\"id_echoed\":\"$echoed\"}}" ;;
    daemon.shutdown) echo "{\"kind\":\"daemon.shutdown\",\"schema_version\":1,\"request_id\":\"$id\"}"; exit 0 ;;
    *) echo "{\"kind\":\"$kind\",\"schema_version\":1,\"request_id\":\"$id\",\"data\":{\"bridged\":$bridged}}" ;;
  esac
done
"#
        .replace("__STORE__", native_id())
        .replace("__SERVICE__", SERVICE);
        kassiber_test_support::write_executable(&path, script).unwrap();
        let launch = Launch {
            data_root: Some(dir.join("data")),
            explicit_root: true,
            bin_flag: Some(path.clone()),
            binary: path,
            env: Vec::new(),
        };
        let mut sup = Supervisor::new(launch);
        let get = sup.alloc_id();
        sup.request(&get, "ai.providers.get", &json!({"name": "remote"}))
            .unwrap();
        // (The stub reads its stdin in order; the real daemon matches the answer by id.)
        let mut got = Vec::new();
        let mut collect = |sup: &mut Supervisor, n: usize| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && got.len() < n {
                for (_, m) in sup.drain() {
                    if let Incoming::Record {
                        request_id,
                        envelope,
                    } = m
                    {
                        got.push((request_id, envelope));
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        collect(&mut sup, 1);
        let plain = sup.alloc_id();
        sup.request(&plain, "ui.overview.snapshot", &Value::Null)
            .unwrap();
        collect(&mut sup, 2);
        let find = |id: &str| {
            got.iter()
                .find(|(r, _)| r == id)
                .map(|(_, e)| e.clone())
                .expect("record")
        };
        let answer = find(&get);
        assert_eq!(answer["data"]["bridged"], true, "{answer}");
        assert_eq!(answer["data"]["state"], "ok", "{answer}");
        assert_eq!(answer["data"]["secret_seen"], "yes");
        assert_eq!(answer["data"]["id_echoed"], "yes");
        assert_eq!(find(&plain)["data"]["bridged"], false);
        drop(sup);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn provider_lists_gain_the_policy_status() {
        let mut env = json!({"kind": "ai.providers.list", "data": {"providers": []}});
        attach_policy_status(&mut env);
        assert!(env["data"]["secret_store_policy"]["default"]["store_id"].is_string());
        let mut other = json!({"kind": "ai.providers.get", "data": {}});
        attach_policy_status(&mut other);
        assert!(other["data"].get("secret_store_policy").is_none());
    }
}
