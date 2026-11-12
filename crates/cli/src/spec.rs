//! The shapes of the generated argparse tree (`spec_data.rs`).
//!
//! Each type mirrors the argparse object it was generated from, field for
//! field, so the engine in `argparse.rs` can follow CPython's algorithm.

/// A Python literal: an action's `default` or `const`, or a parser default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lit {
    /// `argparse.SUPPRESS`: the destination is never set.
    Suppress,
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'static str),
    List(&'static [Lit]),
}

/// The argparse action class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Store,
    StoreConst,
    StoreTrue,
    StoreFalse,
    Append,
    BooleanOptional,
    Help,
    Version,
    SubParsers,
}

/// `nargs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nargs {
    /// `None`: exactly one value.
    Single,
    /// `0`: a flag.
    Zero,
    /// `'?'`
    Optional,
    /// `'*'`
    ZeroOrMore,
    /// `'+'`
    OneOrMore,
    /// `argparse.PARSER`: a subcommand and everything after it.
    Parser,
}

/// The `type=` callable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    /// No type: the string itself.
    Str,
    Int,
    Float,
    /// `kassiber.cli.main._lightning_window_days`: an int from 1 to 365.
    LightningWindowDays,
}

/// An action's `help`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Help {
    Suppressed,
    Missing,
    Text(&'static str),
    /// `--tax-country`: Python's text with the registry's countries, so a
    /// country only the Rust backend supports is listed (preview ones
    /// marked).
    TaxCountries,
    /// `--cost-basis-pool-scope`: each country's scopes and default.
    PoolScopes,
    /// `ai providers create --kind`: Python's text with the Rust-only
    /// `cli_proxy` kind.
    AiProviderKind,
    /// `ai providers update --kind`: likewise.
    AiProviderKindChange,
}

impl Help {
    pub fn text(self) -> Option<&'static str> {
        match self {
            Help::Text(text) => Some(text),
            Help::TaxCountries => Some(tax_countries_help()),
            Help::PoolScopes => Some(pool_scopes_help()),
            Help::AiProviderKind => Some(AI_PROVIDER_KIND_HELP),
            Help::AiProviderKindChange => Some(AI_PROVIDER_KIND_CHANGE_HELP),
            _ => None,
        }
    }
}

/// `ai providers create --kind` (docs/reference/rust-shell.md#cli-proxy-providers):
/// the privacy kinds, and a CLI proxy, which only the Rust backend serves.
pub const AI_PROVIDER_KIND_HELP: &str = "local = on-machine, remote = data leaves the device, tee = encrypted attestation provider, cli_proxy = a CLIProxyAPI server serving your signed-in subscriptions (Rust only; needs --acknowledge, no API format or store choice). Default: local for a loopback URL, else remote (tee and cli_proxy are never inferred; remote still needs --acknowledge)";

/// `ai providers update --kind`.
pub const AI_PROVIDER_KIND_CHANGE_HELP: &str = "Change the privacy kind, or make the provider a cli_proxy (Rust only). Unchanged by default, except that a local provider given a non-loopback --base-url becomes remote; leaving local always needs --acknowledge again";

/// Leaves only the Rust backend serves; Python's grammar lacks them
/// (`scripts/gen_parser_spec.py::RUST_ONLY_LEAVES` grafts them on).
pub const RUST_ONLY_LEAVES: &[&str] = &["wallets.sync-bitpanda"];
/// Backend kinds only the Rust backend accepts, extra `--kind` choices of
/// `backends create` and `backends update`.
pub const RUST_ONLY_BACKEND_KINDS: &[&str] = &["bitpanda"];

/// The `--cost-basis-pool-scope` help: each country's scopes, its default
/// when it has a choice.
pub fn pool_scopes_help() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let countries: Vec<String> = kassiber_tax_countries::COUNTRIES
            .iter()
            .map(|country| match country.pool_scopes {
                [only] => format!("{}: {only}", country.code),
                scopes => format!(
                    "{}: {}, default {}",
                    country.code,
                    scopes.join(" or "),
                    country.default_pool_scope
                ),
            })
            .collect();
        format!(
            "Country-policy-controlled cost-basis pool scope ({}).",
            countries.join("; ")
        )
    })
}

/// The `--tax-country` help over the registry's countries, in picker order.
pub fn tax_countries_help() -> &'static str {
    static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TEXT.get_or_init(|| {
        let countries: Vec<String> = kassiber_tax_countries::COUNTRIES
            .iter()
            .map(|country| {
                if country.preview {
                    format!("{} (preview)", country.code)
                } else {
                    country.code.to_owned()
                }
            })
            .collect();
        format!(
            "Tax country for the book (CLI profile; currently supported: {})",
            countries.join(", ")
        )
    })
}

/// The help pseudo-action argparse records for `add_parser(name, help=...)`.
#[derive(Debug)]
pub struct ChoiceHelp {
    pub dest: &'static str,
    pub metavar: &'static str,
    pub help: Help,
}

/// A subparsers action's choices: names (aliases included) in map order,
/// each with its parser's index in `PARSERS`.
#[derive(Debug)]
pub struct SubParsers {
    pub names: &'static [(&'static str, u16)],
    pub choice_help: &'static [ChoiceHelp],
}

/// One `add_argument` (or `add_subparsers`) action.
#[derive(Debug)]
pub struct Action {
    pub kind: ActionKind,
    pub option_strings: &'static [&'static str],
    /// `None` is `argparse.SUPPRESS`.
    pub dest: Option<&'static str>,
    pub nargs: Nargs,
    pub constant: Lit,
    pub default: Lit,
    pub value_type: ValueType,
    pub required: bool,
    pub help: Help,
    pub metavar: Option<&'static str>,
    pub choices: Option<&'static [&'static str]>,
    pub subparsers: Option<&'static SubParsers>,
}

impl Action {
    pub fn is_positional(&self) -> bool {
        self.option_strings.is_empty()
    }
}

/// A mutually exclusive group, as indices into the parser's actions.
#[derive(Debug)]
pub struct Mutex {
    pub required: bool,
    pub actions: &'static [usize],
}

/// A help section (argparse's action groups).
#[derive(Debug)]
pub struct Section {
    pub title: &'static str,
    pub description: Option<&'static str>,
    pub actions: &'static [usize],
}

/// One `ArgumentParser`.
#[derive(Debug)]
pub struct Parser {
    pub prog: &'static str,
    pub description: Option<&'static str>,
    pub actions: &'static [Action],
    /// `_option_string_actions` in insertion order: option string and
    /// action index.
    pub options: &'static [(&'static str, usize)],
    pub mutex_groups: &'static [Mutex],
    pub sections: &'static [Section],
    /// `set_defaults(...)`.
    pub defaults: &'static [(&'static str, Lit)],
}

impl Parser {
    pub fn option(&self, text: &str) -> Option<usize> {
        self.options
            .iter()
            .find(|(option, _)| *option == text)
            .map(|&(_, index)| index)
    }

    /// The subparsers action, if this parser has subcommands.
    pub fn subparsers(&self) -> Option<&'static SubParsers> {
        self.actions.iter().find_map(|action| action.subparsers)
    }
}

/// The generated tree; index 0 is the `kassiber` parser.
pub fn parsers() -> &'static [Parser] {
    &crate::spec_data::PARSERS
}

/// A parser's index in [`parsers`].
pub type ParserId = usize;
