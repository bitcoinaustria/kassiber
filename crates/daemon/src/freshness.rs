use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
    consent: Arc<Consent>,
    worker: JoinHandle<Finished>,
}

struct Finished {
    response: Value,
    closing: Option<Box<dyn Book>>,
    graph_grants: Vec<Grant>,
}

#[derive(Default)]
pub(crate) struct Requests {
    closing: Option<Box<dyn Book>>,
    active: Option<Active>,
    used: HashSet<String>,
    pending: HashMap<String, Instant>,
    published: Pending,
}

fn busy() -> AppError {
    AppError::new(
        "project_operation_in_progress",
        "Another refresh is still running",
    )
    .with_hint("Wait for the current refresh to finish, then retry.")
    .retryable(true)
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    get(value, key).and_then(|v| match v {
        Value::Str(v) => v.as_str().filter(|v| !v.is_empty()),
        _ => None,
    })
}

impl Requests {
    pub(crate) fn graph_pending(&self) -> usize {
        self.published.len()
    }

    fn publish(&mut self, request_id: &str, grants: Vec<Grant>, _written: Written) {
        self.published.publish(request_id, grants);
    }

    pub(crate) fn running(&self) -> bool {
        self.active.is_some()
    }

    fn take(&mut self, stop: bool) -> Option<Active> {
        if stop {
            if let Some(active) = &self.active {
                active.cancel.cancel();
                active.consent.revoke();
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

    pub(crate) fn control(&mut self, args: &Value, revoke: bool) -> Result<Value, AppError> {
        let target = text(args, "target_request_id").ok_or_else(|| {
            AppError::new("validation", "Freshness control requires target_request_id")
        })?;
        self.pending
            .retain(|_, deadline| *deadline > Instant::now());
        if let Some(active) = self.active.as_ref().filter(|v| v.id == target) {
            if revoke {
                active.consent.revoke();
            } else {
                active.cancel.cancel();
            }
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
    pub(crate) fn reap_freshness(&mut self, stop: bool) -> Result<(), AppError> {
        if stop {
            self.freshness_requests.published.clear();
        }
        if let Some(book) = self.freshness_requests.closing.as_mut() {
            book.close()?;
            self.freshness_requests.closing = None;
        }
        let Some(active) = self.freshness_requests.take(stop) else {
            return Ok(());
        };
        let finished = active.worker.join().unwrap_or_else(|_| Finished {
            response: app_error_envelope(
                &AppError::new("internal_error", "Freshness worker panicked"),
                &RequestId::Value(kassiber_envelope::s(active.id.as_str())),
            ),
            closing: None,
            graph_grants: vec![],
        });
        self.freshness_requests.closing = finished.closing;
        let written = self
            .output
            .write_for_handoff(&finished.response)
            .map_err(|_| AppError::new("cancelled", "Freshness output closed"))?;
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
            "daemon.lock" | "daemon.shutdown" | "ui.profiles.switch"
        ) {
            return self.reap_freshness(true);
        }
        if self.freshness_requests.running()
            && !matches!(
                kind,
                "status"
                    | "ui.network.offline"
                    | "ui.network.offline.set"
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
        let plan = book.freshness_plan(request)?;
        let consent = Arc::new(Consent::explicit_trigger(&plan));
        let cancel = Cancellation::default();
        if get(args, "revoke") == Some(&Value::Bool(true)) {
            consent.revoke();
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
        let output = self.output.clone();
        let progress_id = request_id.clone();
        let worker_cancel = cancel.clone();
        let worker_consent = consent.clone();
        self.freshness_requests.used.insert(id.to_owned());
        let worker = std::thread::Builder::new()
            .name("kassiber-freshness-request".into())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    book.freshness_execute(
                        plan,
                        &options,
                        granted.then_some(worker_consent.as_ref()),
                        &worker_cancel,
                        &mut |event| {
                            output
                                .write(&with_request_id(
                                    success("test.freshness.execute.progress", event.clone()),
                                    &progress_id,
                                ))
                                .map_err(|_| {
                                    worker_cancel.cancel();
                                    worker_consent.revoke();
                                    AppError::new("cancelled", "Freshness output closed")
                                })
                        },
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(AppError::new("internal_error", "Freshness worker panicked"))
                });
                let mut graph_grants = vec![];
                let response = match outcome {
                    Ok(value) => {
                        graph_grants = worker_consent.take_graph_grants();
                        with_request_id(success("test.freshness.execute", value), &progress_id)
                    }
                    Err(error) => {
                        let _ = book.rollback();
                        app_error_envelope(&error, &progress_id)
                    }
                };
                Finished {
                    response,
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
