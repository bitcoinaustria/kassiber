//! Configuration kinds only. Probes and detection stay unimplemented.
pub const KINDS: &[&str] = &[
    "ui.backends.options",
    "ui.backends.public_defaults",
    "ui.backends.settings.list",
    "ui.backends.create",
    "ui.backends.update",
    "ui.backends.delete",
    "ui.backends.set_default",
];
pub fn execute(
    book: &mut dyn kassiber_runtime::store::Book,
    kind: &str,
    args: &kassiber_pyjson::Value,
) -> Result<kassiber_pyjson::Value, kassiber_envelope::AppError> {
    book.service(kind, args)
}
