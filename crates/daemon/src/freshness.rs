pub(crate) mod product;

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::thread::JoinHandle;
#[cfg(any(test, feature = "test-kinds"))]
use std::time::Duration;
use std::time::Instant;

use kassiber_envelope::{dict, get, success, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::{Book, Unlock};
use kassiber_wallets::freshness::{
    execution::Options,
    graph::{Grant, Pending},
    native::Consent,
    planning::Request,
};
use kassiber_wallets::scan::Cancellation;

use crate::output::{with_request_id, RequestId, Written};
use crate::{app_error_envelope, Daemon};

struct Active {
    id: String,
    cancel: Cancellation,
    consent: Option<Arc<Consent>>,
    worker: JoinHandle<Finished>,
}

struct Finished {
    response: Option<Value>,
    after_reply_error: Option<AppError>,
    closing: Option<Box<dyn Book>>,
    graph_grants: Vec<Grant>,
}

struct GraphActive {
    #[cfg(any(test, feature = "test-kinds"))]
    id: String,
    cancel: Cancellation,
    worker: JoinHandle<GraphFinished>,
}

struct GraphFinished {
    closing: Option<Box<dyn Book>>,
    output_closed: bool,
}

#[derive(Default)]
pub(crate) struct Requests {
    closing: Option<Box<dyn Book>>,
    active: Option<Active>,
    used: HashSet<String>,
    pending: HashMap<String, Instant>,
    published: Pending,
    graph: Option<GraphActive>,
    graph_closing: Option<Box<dyn Book>>,
    graph_enabled: bool,
}

fn busy() -> AppError {
    AppError::new(
        "project_operation_in_progress",
        "Another refresh is still running",
    )
    .with_hint("Wait for the current refresh to finish, then retry.")
    .retryable(true)
}

#[cfg(any(test, feature = "test-kinds"))]
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    get(value, key).and_then(|v| match v {
        Value::Str(v) => v.as_str().filter(|v| !v.is_empty()),
        _ => None,
    })
}

impl Requests {
    #[cfg(feature = "test-kinds")]
    pub(crate) fn graph_pending(&self) -> usize {
        self.published.len()
    }

    fn publish(&mut self, request_id: &str, grants: Vec<Grant>, _written: Written) {
        self.published.publish(request_id, grants);
    }

    pub(crate) fn running(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn polling(&self) -> bool {
        self.running() || self.graph.is_some() || self.graph_enabled && !self.published.is_empty()
    }

    fn take(&mut self, stop: bool) -> Option<Active> {
        if stop {
            if let Some(active) = &self.active {
                active.cancel.cancel();
                if let Some(consent) = &active.consent {
                    consent.revoke();
                }
            }
        }
        if self
            .active
            .as_ref()
            .is_some_and(|v| stop || v.worker.is_finished())
        {
            self.active.take()
        } else {
            None
        }
    }

    #[cfg(any(test, feature = "test-kinds"))]
    pub(crate) fn control(&mut self, args: &Value, revoke: bool) -> Result<Value, AppError> {
        let target = text(args, "target_request_id").ok_or_else(|| {
            AppError::new("validation", "Freshness control requires target_request_id")
        })?;
        self.pending
            .retain(|_, deadline| *deadline > Instant::now());
        if let Some(active) = self.active.as_ref().filter(|v| v.id == target) {
            if revoke {
                if let Some(consent) = &active.consent {
                    consent.revoke();
                }
            } else {
                active.cancel.cancel();
            }
            return Ok(dict([
                ("cancelled", Value::Bool(true)),
                ("queued", Value::Bool(false)),
            ]));
        }
        if let Some(active) = self.graph.as_ref().filter(|v| v.id == target) {
            active.cancel.cancel();
            return Ok(dict([
                ("cancelled", Value::Bool(true)),
                ("queued", Value::Bool(false)),
            ]));
        }
        if self.published.revoke_request(target) {
            return Ok(dict([
                ("cancelled", Value::Bool(true)),
                ("queued", Value::Bool(false)),
            ]));
        }
        let queued = !revoke && !self.used.contains(target);
        if queued {
            if self.pending.len() >= 128 && !self.pending.contains_key(target) {
                return Err(busy());
            }
            self.pending
                .insert(target.to_owned(), Instant::now() + Duration::from_secs(60));
        }
        Ok(dict([
            ("cancelled", Value::Bool(queued)),
            ("queued", Value::Bool(queued)),
        ]))
    }
}

impl Daemon {
    #[cfg(feature = "test-kinds")]
    pub(crate) fn start_graph(&mut self) -> Result<(), AppError> {
        self.freshness_requests.graph_enabled = true;
        self.reap_graph(false)
    }

    fn reap_graph(&mut self, stop: bool) -> Result<(), AppError> {
        if stop {
            self.freshness_requests.graph_enabled = false;
            self.freshness_requests.published.clear();
            if let Some(active) = &self.freshness_requests.graph {
                active.cancel.cancel();
                #[cfg(feature = "test-kinds")]
                if std::env::var("KASSIBER_RUST_TEST_GRAPH_STOP_WITNESS").as_deref() == Ok("1") {
                    let _ = self
                        .output
                        .event("test.freshness.graph.stopping", dict::<&str>([]));
                }
            }
        }
        if let Some(book) = self.freshness_requests.graph_closing.as_mut() {
            book.close()?;
            self.freshness_requests.graph_closing = None;
        }
        if self
            .freshness_requests
            .graph
            .as_ref()
            .is_some_and(|v| stop || v.worker.is_finished())
        {
            let active = self.freshness_requests.graph.take().expect("graph worker");
            let finished = active
                .worker
                .join()
                .map_err(|_| AppError::new("internal_error", "Graph worker panicked"))?;
            self.freshness_requests.graph_closing = finished.closing;
            if finished.output_closed {
                self.freshness_requests.published.clear();
                return Err(AppError::new("cancelled", "Graph output closed"));
            }
            if self.freshness_requests.graph_closing.is_some() {
                return Err(
                    AppError::new("project_in_use", "Graph connection could not close")
                        .retryable(true),
                );
            }
        }
        if stop || !self.freshness_requests.graph_enabled || self.freshness_requests.graph.is_some()
        {
            return Ok(());
        }
        let Some((_id, grant)) = self.freshness_requests.published.take() else {
            return Ok(());
        };
        let cancel = grant.cancellation();
        let worker_cancel = cancel.clone();
        let owner = self.owner.as_ref().ok_or_else(|| {
            AppError::new("project_in_use", "Graph worker requires project ownership")
        })?;
        let identity = owner.project().identity.clone();
        let store = self.store.clone();
        let root = self.data_root.clone();
        let mut config = self.runtime_config.clone();
        let passphrase = self
            .unlocked_passphrase
            .as_ref()
            .map(|v| kassiber_runtime::store::Secret::new(v.expose().to_owned()));
        let output = self.output.clone();
        let worker = std::thread::Builder::new()
            .name("kassiber-graph-references".into())
            .spawn(move || {
                let opened = store.open_book(
                    &root,
                    Unlock::Daemon {
                        passphrase: passphrase.as_ref(),
                        require_existing_schema: true,
                        expected_project_identity: &identity,
                    },
                    &mut config,
                );
                drop(passphrase);
                let mut book = match opened {
                    Ok(book) => book,
                    Err(_) => {
                        let output_closed = output
                            .event(
                                "ui.freshness.worker",
                                dict([
                                    ("status", kassiber_envelope::s("error")),
                                    (
                                        "code",
                                        kassiber_envelope::s("freshness_background_open_failed"),
                                    ),
                                    (
                                        "message",
                                        kassiber_envelope::s(
                                            "Graph worker could not open its book",
                                        ),
                                    ),
                                    ("retryable", Value::Bool(true)),
                                ]),
                            )
                            .is_err();
                        return GraphFinished {
                            closing: None,
                            output_closed,
                        };
                    }
                };
                let result = catch_unwind(AssertUnwindSafe(|| {
                    book.graph_followup(grant, &worker_cancel)
                }))
                .unwrap_or_else(|_| Err(AppError::new("internal_error", "Graph worker panicked")));
                let output_closed = match result {
                    Ok(Some(value)) => output.event("ui.freshness.background", value).is_err(),
                    Ok(None) => false,
                    Err(_) => {
                        let _ = book.rollback();
                        output
                            .event(
                                "ui.freshness.worker",
                                dict([
                                    ("status", kassiber_envelope::s("error")),
                                    ("code", kassiber_envelope::s("freshness_background_error")),
                                    (
                                        "message",
                                        kassiber_envelope::s("Graph reference worker failed"),
                                    ),
                                    ("retryable", Value::Bool(true)),
                                ]),
                            )
                            .is_err()
                    }
                };
                if output_closed {
                    worker_cancel.cancel();
                }
                GraphFinished {
                    closing: if book.close().is_err() {
                        Some(book)
                    } else {
                        None
                    },
                    output_closed,
                }
            })
            .map_err(|_| AppError::new("internal_error", "Could not start graph worker"))?;
        self.freshness_requests.graph = Some(GraphActive {
            #[cfg(any(test, feature = "test-kinds"))]
            id: _id,
            cancel,
            worker,
        });
        Ok(())
    }

    pub(crate) fn reap_freshness(&mut self, stop: bool) -> Result<(), AppError> {
        self.reap_graph(stop)?;
        if let Some(book) = self.freshness_requests.closing.as_mut() {
            book.close()?;
            self.freshness_requests.closing = None;
        }
        let Some(active) = self.freshness_requests.take(stop) else {
            return Ok(());
        };
        let finished = active.worker.join().unwrap_or_else(|_| Finished {
            response: Some(app_error_envelope(
                &AppError::new("internal_error", "Freshness worker panicked"),
                &RequestId::Value(kassiber_envelope::s(active.id.as_str())),
                None,
            )),
            after_reply_error: None,
            closing: None,
            graph_grants: vec![],
        });
        self.freshness_requests.closing = finished.closing;
        let written = if let Some(response) = finished.response {
            self.output
                .write_for_handoff(&response)
                .map_err(|_| AppError::new("cancelled", "Freshness output closed"))?
        } else {
            None
        };
        if let Some(error) = finished.after_reply_error {
            return Err(error);
        }
        if self.freshness_requests.closing.is_some() {
            return Err(
                AppError::new("project_in_use", "Freshness connection could not close")
                    .retryable(true),
            );
        }
        if !stop {
            if let Some(written) = written {
                self.freshness_requests
                    .publish(&active.id, finished.graph_grants, written);
            }
        }
        Ok(())
    }

    pub(crate) fn freshness_before_request(&mut self, kind: &str) -> Result<(), AppError> {
        self.reap_freshness(false)?;
        if matches!(
            kind,
            "daemon.lock"
                | "daemon.shutdown"
                | "ui.profiles.switch"
                | "ui.backup.apply"
                | "ui.projects.create"
                | "ui.projects.select"
        ) {
            return self.reap_freshness(true);
        }
        if self.freshness_requests.running()
            && !matches!(
                kind,
                "status"
                    | "ui.logs.snapshot"
                    | "ui.egress.snapshot"
                    | "ui.network.offline"
                    | "ui.network.offline.set"
                    | "ui.freshness.cancel"
                    | "ui.freshness.pause"
                    | "ui.freshness.resume"
                    | "test.freshness.cancel"
                    | "test.freshness.pause"
                    | "test.freshness.resume"
                    | "test.freshness.request.cancel"
                    | "test.freshness.request.revoke"
                    | "test.freshness.graph.pending"
            )
        {
            return Err(busy());
        }
        Ok(())
    }

    pub(crate) fn start_freshness(
        &mut self,
        kind: &str,
        request_id: &RequestId,
        request: &Request,
        options: Options,
        args: &Value,
    ) -> Result<(), AppError> {
        self.reap_freshness(false)?;
        let RequestId::Value(Value::Str(id)) = request_id else {
            return Err(AppError::new(
                "validation",
                "Freshness execution requires a non-empty string request_id",
            ));
        };
        let id = id.as_str().filter(|v| !v.is_empty()).ok_or_else(|| {
            AppError::new(
                "validation",
                "Freshness execution requires a non-empty string request_id",
            )
        })?;
        if self.freshness_requests.used.contains(id) {
            return Err(AppError::new(
                "interaction_required",
                "Freshness request was already used",
            ));
        }
        if self.freshness_requests.running() {
            return Err(busy());
        }
        if self.freshness_requests.used.len() >= 4096 {
            return Err(AppError::new(
                "interaction_required",
                "Freshness request limit reached; restart the daemon",
            ));
        }
        let journal_context = if kind == "ui.journals.process" {
            Some(self.book_mut()?.context()?)
        } else {
            None
        };
        let owner = self.owner.as_ref().ok_or_else(|| {
            AppError::new("project_in_use", "Freshness requires project ownership")
        })?;
        let mut config = self.runtime_config.clone();
        let mut book = self.store.open_book(
            &self.data_root,
            Unlock::Daemon {
                passphrase: self.unlocked_passphrase.as_ref(),
                require_existing_schema: true,
                expected_project_identity: &owner.project().identity,
            },
            &mut config,
        )?;
        let journals = kind == "ui.journals.process";
        // The desktop action pairs automatically unless the Review screen
        // recalculates after the owner's own decisions.
        let journal_auto_pair = request.auto_pair;
        let plan = if journals {
            None
        } else {
            Some(book.freshness_plan(request)?)
        };
        if kind == "ui.wallets.sync"
            && !plan
                .as_ref()
                .is_some_and(|plan| plan.supports_wallets_sync())
        {
            return Err(AppError::new(
                "native_fallback_required",
                "Wallet sync requires its direct-sync reference path",
            ));
        }
        let consent = plan
            .as_ref()
            .map(|plan| Arc::new(Consent::explicit_trigger(plan)));
        let cancel = Cancellation::default();
        if get(args, "revoke") == Some(&Value::Bool(true)) {
            if let Some(consent) = &consent {
                consent.revoke();
            }
        }
        if get(args, "cancel") == Some(&Value::Bool(true))
            || self
                .freshness_requests
                .pending
                .remove(id)
                .is_some_and(|v| v > Instant::now())
        {
            cancel.cancel();
        }
        let granted = get(args, "consent") != Some(&Value::Bool(false));
        let automatic_graph = !journals && !kind.starts_with("test.");
        let kind = kind.to_owned();
        let output = self.output.clone();
        let progress_id = request_id.clone();
        let worker_cancel = cancel.clone();
        let worker_consent = consent.clone();
        self.freshness_requests.used.insert(id.to_owned());
        let worker = std::thread::Builder::new()
            .name("kassiber-freshness-request".into())
            .spawn(move || {
                let mut response_written = false;
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let mut emit = |event: &Value| {
                        output
                            .write(&with_request_id(
                                success(&format!("{kind}.progress"), event.clone()),
                                &progress_id,
                            ))
                            .map_err(|_| {
                                worker_cancel.cancel();
                                if let Some(consent) = &worker_consent {
                                    consent.revoke();
                                }
                                AppError::new("cancelled", "Freshness output closed")
                            })
                    };
                    if journals {
                        let result = book.refresh_journals(
                            journal_context.as_ref().expect("captured journal context"),
                            journal_auto_pair,
                            &worker_cancel,
                            &mut emit,
                        )?;
                        output
                            .write(&with_request_id(
                                success(&kind, result.payload().clone()),
                                &progress_id,
                            ))
                            .map_err(|_| AppError::new("cancelled", "Freshness output closed"))?;
                        response_written = true;
                        result.after_reply()?;
                        return Ok(None);
                    }
                    book.freshness_execute(
                        plan.expect("freshness plan"),
                        &options,
                        worker_consent.as_deref().filter(|_| granted),
                        &worker_cancel,
                        &mut emit,
                    )
                    .map(Some)
                }))
                .unwrap_or_else(|_| {
                    Err(AppError::new("internal_error", "Freshness worker panicked"))
                });
                let mut graph_grants = vec![];
                let mut after_reply_error = None;
                let response = match outcome {
                    Ok(Some(value)) => {
                        if let Some(consent) = &worker_consent {
                            graph_grants = consent.take_graph_grants();
                        }
                        Some(with_request_id(success(&kind, value), &progress_id))
                    }
                    Ok(None) => None,
                    Err(error) => {
                        let _ = book.rollback();
                        if response_written {
                            after_reply_error = Some(error);
                            None
                        } else {
                            Some(app_error_envelope(&error, &progress_id, Some(&kind)))
                        }
                    }
                };
                Finished {
                    response,
                    after_reply_error,
                    graph_grants,
                    closing: if book.close().is_err() {
                        Some(book)
                    } else {
                        None
                    },
                }
            })
            .map_err(|_| AppError::new("internal_error", "Could not start freshness worker"))?;
        self.freshness_requests.active = Some(Active {
            id: id.into(),
            cancel,
            consent,
            worker,
        });
        if automatic_graph {
            self.freshness_requests.graph_enabled = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_cancels_expire_and_never_rearm_used_requests() {
        let mut requests = Requests::default();
        requests
            .pending
            .insert("expired".into(), Instant::now() - Duration::from_secs(1));
        requests.used.insert("used".into());
        let response = requests
            .control(
                &dict([("target_request_id", kassiber_envelope::s("used"))]),
                false,
            )
            .unwrap();
        assert_eq!(
            response,
            dict([
                ("cancelled", Value::Bool(false)),
                ("queued", Value::Bool(false))
            ])
        );
        assert!(requests.pending.is_empty());
        assert_eq!(
            requests
                .control(
                    &dict([("target_request_id", kassiber_envelope::s("future"))]),
                    true
                )
                .unwrap(),
            response
        );
        assert!(requests.pending.is_empty());
    }

    #[test]
    fn early_cancel_capacity_preserves_existing_fences() {
        let mut requests = Requests::default();
        for index in 0..128 {
            requests
                .control(
                    &dict([("target_request_id", kassiber_envelope::s(index.to_string()))]),
                    false,
                )
                .unwrap();
        }
        assert_eq!(
            requests
                .control(
                    &dict([("target_request_id", kassiber_envelope::s("overflow"))]),
                    false
                )
                .unwrap_err()
                .code,
            "project_operation_in_progress"
        );
        requests
            .control(
                &dict([("target_request_id", kassiber_envelope::s("0"))]),
                false,
            )
            .unwrap();
        assert_eq!(requests.pending.len(), 128);
        assert!(requests.pending.contains_key("0"));
    }
}
