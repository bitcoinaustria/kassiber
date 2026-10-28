//! Wallet family registration. Unsupported wallet operations stay disabled.
use crate::argparse::Namespace;
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub fn execute(namespace: &Namespace, book: &mut dyn Book) -> Result<Value, AppError> {
    let (_, profile_id) =
        book.resolve_scope(namespace.get("workspace"), namespace.get("profile"))?;
    book.derive_wallet(kassiber_wallets::Derive {
        profile_id,
        wallet: namespace.text("wallet").unwrap_or_default().to_owned(),
        branch: namespace.text("branch").map(str::to_owned),
        start: namespace.get("start").cloned().unwrap_or(Value::from(0)),
        count: namespace.get("count").cloned(),
    })
}
