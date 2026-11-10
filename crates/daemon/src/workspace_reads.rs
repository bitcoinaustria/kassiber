//! Workspace shell reads: overviews, health, next actions and review badges.
//! The no-argument reads ignore their payload as Python does; the book-set
//! overview passes it through for its own argument validation.
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const OVERVIEW_KINDS: &[&str] = &["ui.overview.snapshot", "ui.workspace.overview.snapshot"];
pub const HEALTH_KIND: &str = "ui.workspace.health";
pub const ACTION_KINDS: &[&str] = &["ui.next_actions", "ui.review.badges"];
pub const ONBOARDING_KIND: &str = "ui.onboarding.complete";

pub fn handles(kind: &str) -> bool {
    OVERVIEW_KINDS.contains(&kind)
        || kind == HEALTH_KIND
        || ACTION_KINDS.contains(&kind)
        || kind == ONBOARDING_KIND
}

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    if kind == ONBOARDING_KIND {
        let args = match args {
            None | Some(Value::Null) => dict::<&str>([]),
            Some(args @ Value::Dict(_)) => args.clone(),
            Some(other) => {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([(
                        "type",
                        kassiber_envelope::s(kassiber_envelope::type_name(other)),
                    )])))
            }
        };
        return book.service(kind, &args);
    }
    if kind == "ui.workspace.overview.snapshot" {
        return book.service(kind, args.unwrap_or(&Value::Null));
    }
    book.service(kind, &dict::<&str>([]))
}
