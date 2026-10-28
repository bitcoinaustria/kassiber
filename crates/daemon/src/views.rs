//! views daemon registration. Use the shared setup service.
pub const KINDS: &[&str] = &[
    "ui.saved_views.list",
    "ui.saved_views.create",
    "ui.saved_views.delete",
];
pub fn execute(
    book: &mut dyn kassiber_runtime::store::Book,
    kind: &str,
    args: &kassiber_pyjson::Value,
) -> Result<kassiber_pyjson::Value, kassiber_envelope::AppError> {
    book.service(kind, args)
}
