//! The AI provider kinds (R6-6a): `ai.providers.*`,
//! `ai.provider_runtime.status`, `ai.list_models` and `ai.test_connection`.
//! The provider store and the HTTP client live in `kassiber-ai`; this
//! module lends them the book, the desktop's keychain over the control
//! channel, and the daemon's RAM-only discovery cache.

use std::sync::Arc;

use kassiber_ai::discovery::Cache;
use kassiber_envelope::{success, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::AiSurface;

use crate::control::{
    secret_store_request, BridgeError, SecretStoreRequest, SECRET_STORE_BRIDGE_TIMEOUT,
};
use crate::input::Input;
use crate::output::{with_request_id, Output, RequestId};
use crate::{Daemon, Failure, Handled};

pub const KINDS: &[&str] = kassiber_ai::service::KINDS;

struct Host<'a> {
    output: Output,
    input: &'a mut Input,
    cache: Arc<Cache>,
    /// Stdout closed during a keychain round trip: the daemon stops.
    closed: bool,
}

impl kassiber_ai::service::Host for Host<'_> {
    fn secret_store(
        &mut self,
        op: &str,
        provider_name: &str,
        store_id: &str,
        service: &str,
        account: &str,
        secret: Option<&str>,
    ) -> Result<Value, AppError> {
        let request = SecretStoreRequest {
            op,
            provider_name,
            store_id,
            service,
            account,
            secret,
        };
        match secret_store_request(
            &self.output,
            self.input,
            &request,
            SECRET_STORE_BRIDGE_TIMEOUT,
        ) {
            Ok(data) => Ok(data),
            Err(BridgeError::App(error)) => Err(error),
            Err(BridgeError::Closed(_)) => {
                self.closed = true;
                Err(AppError::new("internal_error", "daemon output closed"))
            }
        }
    }

    fn discovery(&self) -> &Cache {
        &self.cache
    }

    fn gate(&self) -> kassiber_ai::service::Gate {
        kassiber_ai::service::system_gate()
    }
}

pub(crate) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
    input: &mut Input,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let mut host = Host {
        output: daemon.output.clone(),
        input,
        cache: Arc::clone(&daemon.ai_discovery),
        closed: false,
    };
    let result = daemon
        .book_mut()?
        .ai_providers(AiSurface::Daemon, kind, args, &mut host);
    if host.closed {
        return Err(Failure::Closed);
    }
    Ok((
        Some(with_request_id(success(kind, result?), request_id)),
        false,
    ))
}
