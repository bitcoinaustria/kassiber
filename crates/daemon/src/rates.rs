//! Desktop rates kinds use the same service boundary as the CLI.
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub(crate) const KINDS: &[&str] = &[
    "ui.rates.summary",
    "ui.rates.coverage",
    "ui.rates.kraken_csv.import",
    "ui.rates.latest",
    "ui.rates.rebuild",
];
pub(crate) fn run(
    book: &mut dyn Book,
    kind: &str,
    args: Option<&Value>,
) -> Result<Value, AppError> {
    let empty = kassiber_envelope::dict::<&str>([]);
    let args = args.unwrap_or(&empty);
    if !matches!(args, Value::Null | Value::Dict(_)) {
        let error = AppError::new("validation", "daemon args must be an object");
        if kind == "ui.rates.rebuild" {
            return Err(error.with_details(kassiber_envelope::dict([(
                "type",
                kassiber_envelope::s(kassiber_envelope::type_name(args)),
            )])));
        }
        return Err(error);
    }
    book.rates(kind, args)
}
