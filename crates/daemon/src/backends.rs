//! Local configuration and explicitly requested consent-bound probes.
pub const KINDS: &[&str] = &[
    "ui.backends.list",
    "ui.backends.options",
    "ui.backends.public_defaults",
    "ui.backends.settings.list",
    "ui.backends.create",
    "ui.backends.update",
    "ui.backends.delete",
    "ui.backends.set_default",
    "ui.backends.bitcoinrpc.test",
    "ui.backends.btcpay.test",
    "ui.backends.detect_core",
    "ui.backends.electrum.test",
    "ui.backends.http.test",
    "ui.backends.lightning.test",
    "backends.reveal_token",
];
pub fn execute(
    book: &mut dyn kassiber_runtime::store::Book,
    kind: &str,
    args: &kassiber_pyjson::Value,
) -> Result<kassiber_pyjson::Value, kassiber_envelope::AppError> {
    if kind == "backends.reveal_token" {
        let target = ["name", "wallet", "backend"]
            .into_iter()
            .filter_map(|k| kassiber_envelope::get(args, k))
            .find(|v| kassiber_envelope::truthy(v))
            .cloned()
            .unwrap_or(kassiber_pyjson::Value::Null);
        book.service(
            "backends.reveal-token",
            &kassiber_envelope::dict([("name", target)]),
        )
    } else {
        book.service(kind, args)
    }
}
