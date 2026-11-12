//! `sync_progress_emitter` for record imports: a caller that streams
//! progress (a freshness job, a desktop wallet sync) collects the events an
//! import emits on this thread and publishes them once the import's own
//! savepoint is closed, in order.
use kassiber_pyjson::Value;
use std::cell::RefCell;

thread_local! {
    static EVENTS: RefCell<Option<Vec<Value>>> = const { RefCell::new(None) };
    static WALLET: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Whether a caller is collecting, so emitters can skip building events.
pub(crate) fn active() -> bool {
    EVENTS.with(|events| events.borrow().is_some())
}

/// Queue one event for the collecting caller, if any.
pub(crate) fn emit(event: Value) {
    let event = WALLET.with(|wallet| match wallet.borrow().as_ref() {
        // `_wrap_sync_progress_for_wallet`: `{"wallet": label, **payload}`.
        Some(label) => {
            let mut wrapped = kassiber_envelope::dict([("wallet", label.clone())]);
            if let Value::Dict(pairs) = event {
                for (key, value) in pairs {
                    if let kassiber_pyjson::Key::Str(name) = &key {
                        let name = kassiber_envelope::py_str(&Value::Str(name.clone()));
                        kassiber_envelope::set(&mut wrapped, &name, value);
                    }
                }
            }
            wrapped
        }
        None => event,
    });
    EVENTS.with(|events| {
        if let Some(events) = events.borrow_mut().as_mut() {
            events.push(event);
        }
    });
}

/// Run `f` with every event it emits labelled by `wallet`, as
/// `_wrap_sync_progress_for_wallet` does for BTCPay and node routes.
pub(crate) fn for_wallet<R>(wallet: &Value, f: impl FnOnce() -> R) -> R {
    let previous = WALLET.with(|label| label.borrow_mut().replace(wallet.clone()));
    let result = f();
    WALLET.with(|label| *label.borrow_mut() = previous);
    result
}

/// Run `f`, collecting the events it emits; an enclosing collection resumes
/// afterwards.
pub(crate) fn collect<R>(f: impl FnOnce() -> R) -> (R, Vec<Value>) {
    let previous = EVENTS.with(|events| events.borrow_mut().replace(Vec::new()));
    struct Restore(Option<Option<Vec<Value>>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                EVENTS.with(|events| *events.borrow_mut() = previous);
            }
        }
    }
    let mut restore = Restore(Some(previous));
    let result = f();
    let collected = EVENTS
        .with(|events| events.borrow_mut().take())
        .unwrap_or_default();
    if let Some(previous) = restore.0.take() {
        EVENTS.with(|events| *events.borrow_mut() = previous);
    }
    drop(restore);
    (result, collected)
}

/// Run `f` with no collector, as Python applies a chain wallet's fetch
/// after `sync_progress_emitter` is reset: its imports emit nothing.
pub(crate) fn silenced<R>(f: impl FnOnce() -> R) -> R {
    let previous = EVENTS.with(|events| events.borrow_mut().take());
    struct Restore(Option<Option<Vec<Value>>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                EVENTS.with(|events| *events.borrow_mut() = previous);
            }
        }
    }
    let _restore = Restore(Some(previous));
    f()
}
