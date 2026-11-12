//! Desktop network review uses the same book services as the CLI.
pub const KINDS: &[&str] = &[
    "ui.networks.partition_plan",
    "ui.networks.partition_export",
    "ui.networks.binding",
    "ui.networks.inventory",
    "ui.networks.plan",
    "ui.networks.bind",
];
pub fn execute(
    book: &mut dyn kassiber_runtime::store::Book,
    kind: &str,
    args: &kassiber_pyjson::Value,
) -> Result<kassiber_pyjson::Value, kassiber_envelope::AppError> {
    book.service(kind, args)
}
