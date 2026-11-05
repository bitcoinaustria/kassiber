//! In-product chat (R6-7): `ai.chat` and its controls, chat history, and
//! the pieces `kassiber chat` shares with the daemon. The daemon registers
//! the chat kinds and the CLI its leaves only behind the chat gate (design
//! §7 item 1): chat stays out of product builds until every tool it can
//! call is native.

pub mod args;
pub mod client;
pub mod history;
pub mod registry;
pub mod screen;
pub mod turn;

pub use args::{validate, Validated};
pub use client::{ChatClient, ProviderSnapshot};
pub use registry::{registry_key, ActiveChat, ActiveChats};
pub use turn::{Turn, TurnHost, Worker};
