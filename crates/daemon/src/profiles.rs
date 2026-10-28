//! profiles daemon registration. Use the shared setup service.
pub const KINDS: &[&str] = &[
    "ui.profiles.snapshot",
    "ui.profiles.create",
    "ui.profiles.rename",
    "ui.profiles.update",
    "ui.profiles.switch",
    "ui.profiles.reset_data",
];
pub fn execute(
    book: &mut dyn kassiber_runtime::store::Book,
    kind: &str,
    args: &kassiber_pyjson::Value,
) -> Result<kassiber_pyjson::Value, kassiber_envelope::AppError> {
    book.service(kind, args)
}
