//! `ai_client_for_locator(...)` for a chat turn: the HTTP client for an
//! OpenAI-compatible endpoint, or the provider broker for a `*-cli://`
//! locator, behind the one interface the chat loop drives.

use std::sync::Arc;
use std::time::Duration;

use kassiber_envelope::AppError;
use kassiber_net::Gate;
use kassiber_pyjson::Value;

use crate::broker::{BrokerChat, BrokerClient, Locate};
use crate::providers::is_cli_locator;
use crate::pyval::{field, s, string};
use crate::transport::{ChatDelta, ChatRequest, ClientConfig, HttpClient, Receipt, RequestContext};

/// What `ai.chat` resolved about its provider on the request loop
/// (`provider_snapshot`). The key never leaves this process except in the
/// `Authorization` header.
#[derive(Clone)]
pub struct ProviderSnapshot {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub kind: Option<String>,
    pub api_format: Option<String>,
    pub detected_api_format: Option<String>,
    pub omit_store_field: bool,
    /// `_provider_is_on_device(provider)`: local kind and a loopback URL.
    pub on_device: bool,
    /// `_provider_has_full_agent_access(provider)`: on this device, or an
    /// off-device provider the user did not restrict (#678).
    pub full_access: bool,
}

impl ProviderSnapshot {
    /// From the dict `internal.ai.chat.provider` answers.
    pub fn from_value(value: &Value) -> Self {
        let text = |key: &str| string(field(value, key));
        ProviderSnapshot {
            name: text("name").unwrap_or_default(),
            base_url: text("base_url").unwrap_or_default(),
            api_key: text("api_key"),
            kind: text("kind"),
            api_format: text("api_format"),
            detected_api_format: text("detected_api_format"),
            omit_store_field: kassiber_envelope::truthy(field(value, "omit_store_field")),
            on_device: kassiber_envelope::truthy(field(value, "on_device")),
            full_access: kassiber_envelope::truthy(field(value, "full_access")),
        }
    }

    /// The fields `_ai_answer_provenance` reads.
    pub fn provenance_view(&self) -> Value {
        kassiber_envelope::dict([
            ("name", s(&self.name)),
            ("kind", self.kind.as_deref().map_or(Value::Null, s)),
        ])
    }
}

/// Stops a client from another thread.
pub type Canceller = Arc<dyn Fn() + Send + Sync>;

/// The turn's provider client; one per turn, so the size difference
/// between the two clients does not matter.
#[allow(clippy::large_enum_variant)]
pub enum ChatClient {
    Http(HttpClient),
    Broker(BrokerClient),
}

impl ChatClient {
    /// `ai_client_for_locator(base_url, api_key=..., timeout=..., kind=...,
    /// api_format=..., detected_api_format=..., omit_store_field=...)`.
    /// HTTP requests are counted into `receipt`; the broker records none,
    /// as Python's ledger records none (design §7 item 3).
    pub fn for_provider(
        provider: &ProviderSnapshot,
        timeout: Duration,
        gate: Gate,
        locate: Locate,
        receipt: Receipt,
    ) -> Result<Self, AppError> {
        if is_cli_locator(&s(&provider.base_url)) {
            return Ok(ChatClient::Broker(
                BrokerClient::new(&provider.base_url, timeout, gate)?.with_locate(locate),
            ));
        }
        let mut config = ClientConfig::new(provider.base_url.clone());
        config.api_key = provider.api_key.clone();
        config.timeout = timeout;
        config.on_device = provider.kind.as_deref() == Some("local");
        config.api_format = provider
            .api_format
            .clone()
            .filter(|format| !format.is_empty())
            .unwrap_or_else(|| crate::providers::API_FORMAT_AUTO.to_owned());
        config.detected_api_format = provider.detected_api_format.clone();
        config.omit_store_field = provider.omit_store_field;
        Ok(ChatClient::Http(
            HttpClient::new(config, gate).with_receipt(receipt),
        ))
    }

    /// `client.cancel`: shuts the request or the broker down, and revokes
    /// every network consent the turn was granted.
    pub fn canceller(&self) -> Canceller {
        match self {
            ChatClient::Http(client) => {
                let cancel = client.canceller();
                let revocation = client.revocation();
                Arc::new(move || {
                    revocation.revoke();
                    cancel.cancel();
                })
            }
            ChatClient::Broker(client) => {
                let cancel = client.canceller();
                Arc::new(move || cancel.cancel())
            }
        }
    }

    /// The handle that revokes an HTTP client's consents (the broker's
    /// process route is stopped by its own cancel).
    pub fn revocation(&self) -> Option<crate::transport::Revocation> {
        match self {
            ChatClient::Http(client) => Some(client.revocation()),
            ChatClient::Broker(_) => None,
        }
    }

    /// `stream_chat(messages=..., model=..., options=...)`: the tool-free
    /// path.
    pub fn stream_messages(
        &mut self,
        messages: &[Value],
        model: &str,
        options: &Value,
        emit: &mut dyn FnMut(ChatDelta) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        match self {
            ChatClient::Http(client) => {
                let context = RequestContext::from_messages(messages);
                client.stream_chat(
                    &ChatRequest {
                        context: &context,
                        model,
                        options,
                        tools: None,
                        tool_choice: &Value::Null,
                    },
                    emit,
                )
            }
            ChatClient::Broker(client) => client.stream_chat(
                &BrokerChat {
                    messages: Some(messages),
                    context: None,
                    model,
                    options,
                    tools: &[],
                },
                emit,
            ),
        }
    }

    /// `stream_chat(model=..., options=..., tools=..., tool_choice="auto",
    /// context=...)`: one leg of the tool loop.
    pub fn stream_tools(
        &mut self,
        context: &RequestContext,
        model: &str,
        options: &Value,
        tools: &[Value],
        emit: &mut dyn FnMut(ChatDelta) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        match self {
            ChatClient::Http(client) => client.stream_chat(
                &ChatRequest {
                    context,
                    model,
                    options,
                    tools: Some(tools),
                    tool_choice: &s("auto"),
                },
                emit,
            ),
            ChatClient::Broker(client) => client.stream_chat(
                &BrokerChat {
                    messages: None,
                    context: Some(context),
                    model,
                    options,
                    tools,
                },
                emit,
            ),
        }
    }

    /// `getattr(client, "last_provider_session_id", None)`.
    pub fn last_provider_session_id(&self) -> Option<String> {
        match self {
            ChatClient::Http(_) => None,
            ChatClient::Broker(client) => string(&client.last_provider_session_id),
        }
    }

    /// `client.newly_detected_api_format`, cleared once read.
    pub fn take_detected_api_format(&mut self) -> Option<&'static str> {
        match self {
            ChatClient::Http(client) => client.newly_detected_api_format.take(),
            ChatClient::Broker(_) => None,
        }
    }
}
