//! Kassiber's PDF renderers. They receive finished report models: no tax,
//! journal or custody decision is made here.
//!
//! `text` ports the hand-written PDF 1.4 report writer byte for byte.
//! `engine` lays out the shared report design with a sealed Typst world.
pub mod engine;
pub mod text;
