//! `kassiber commands describe`: the command catalog from the Rust tree.
//!
//! Each entry is computed the way `cli/command_registry.py` computes it
//! from argparse (arguments, effect, capability, `needs_database`, scope
//! flags, `secret_input`, `may_prompt`). Metadata covers every Python leaf;
//! execution support is tracked separately by `IMPLEMENTED`. A query that
//! matches nothing fails `unknown_command`, as in Python.

use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::capabilities::{cli_capability, Capability};

use crate::spec::{parsers, ActionKind, Help, Nargs, ParserId};

/// The leaves this binary implements, as dotted command paths.
pub const IMPLEMENTED: &[&str] = &[
    "update",
    "verify-download",
    "health",
    "next-actions",
    "ai.providers.list",
    "ai.providers.get",
    "ai.providers.create",
    "ai.providers.update",
    "ai.providers.delete",
    "ai.providers.set-default",
    "ai.providers.clear-default",
    "ai.providers.test",
    "ai.models",
    "wallets.sync",
    "journals.process",
    "diagnostics.collect",
    "backends.list",
    "backends.kinds",
    "backends.get",
    "backends.create",
    "backends.update",
    "backends.delete",
    "backends.set-default",
    "backends.clear-default",
    "backends.reveal-token",
    "secrets.init",
    "backup.export",
    "backup.import",
    "secrets.init-resume",
    "secrets.verify",
    "secrets.change-passphrase",
    "secrets.remember-unlock",
    "secrets.forget-unlock",
    "secrets.status",
    "secrets.migrate-credentials",
    "init",
    "daemon",
    "status",
    "commands.describe",
    "rates.pairs",
    "rates.set",
    "rates.latest",
    "rates.range",
    "rates.sync",
    "rates.rebuild",
    "offline.status",
    "offline.on",
    "offline.off",
    "projects.list",
    "projects.show",
    "projects.create",
    "projects.select",
    "workspaces.list",
    "workspaces.create",
    "profiles.list",
    "profiles.create",
    "profiles.get",
    "profiles.set",
    "accounts.list",
    "accounts.create",
    "context.show",
    "context.current",
    "context.set",
    "views.list",
    "views.create",
    "views.delete",
    "networks.inventory",
    "networks.plan",
    "networks.bind",
    "networks.split-plan",
    "networks.split",
    "transactions.list",
    "loans.mark",
    "loans.unmark",
    "loans.link",
    "loans.list",
    "documents.list",
    "documents.create",
    "documents.attach",
    "documents.import-report",
    "btcpay.provenance.suggest",
    "btcpay.provenance.list",
    "btcpay.provenance.links",
    "attachments.add",
    "attachments.list",
    "attachments.rename",
    "attachments.remove",
    "attachments.verify",
    "attachments.gc",
    "metadata.bip329.preview",
    "metadata.bip329.import",
    "metadata.bip329.list",
    "metadata.bip329.export",
    "metadata.tags.list",
    "metadata.tags.create",
    "metadata.tags.add",
    "metadata.tags.remove",
    "metadata.notes.set",
    "metadata.notes.clear",
    "metadata.exclude",
    "metadata.include",
    "metadata.records.get",
    "metadata.records.note.set",
    "metadata.records.note.clear",
    "metadata.records.tag.add",
    "metadata.records.tag.remove",
    "metadata.records.excluded.set",
    "metadata.records.excluded.clear",
    "metadata.records.kind.set",
    "metadata.records.kind.clear",
    "metadata.records.history.stale",
    "metadata.records.list",
    "metadata.records.history.list",
    "metadata.records.history.activity",
    "metadata.records.history.revert",
    "imports.list",
    "imports.rollback",
    "wallets.derive",
    "wallets.import-csv",
    "wallets.import-json",
    "wallets.import-river",
    "wallets.import-bull",
    "wallets.import-bullbitcoin",
    "wallets.import-coinfinity",
    "wallets.import-21bitcoin",
    "wallets.import-strike",
    "wallets.import-ledger-live",
    "wallets.import-cointracking",
    "wallets.import-blockpit",
    "wallets.import-binance-supplemental",
    "wallets.import-pocket",
    "wallets.import-pocketbitcoin",
    "wallets.import-phoenix",
    "wallets.create",
    "wallets.list",
    "wallets.get",
    "wallets.update",
    "wallets.delete",
    "reports.summary",
    "reports.commercial-subledger",
    "reports.export-commercial-subledger-csv",
    "reports.balance-sheet",
    "reports.portfolio-summary",
    "reports.capital-gains",
    "reports.explain-capital-gain",
    "reports.filed-snapshots.list",
    "reports.filed-snapshots.create",
    "reports.tax-summary",
    "reports.journal-entries",
    "reports.austrian-e1kv",
    "reports.austrian-tax-summary",
    "reports.balance-history",
    "transfers.list",
    "transfers.suggest",
    "transfers.bulk-pair",
    "transfers.rules.apply",
    "transfers.pair",
    "transfers.unpair",
    "transfers.update",
    "transfers.payouts.list",
    "transfers.payouts.delete",
    "transfers.payouts.create",
    "transfers.gaps.history",
    "transfers.gaps.plan",
    "transfers.gaps.apply",
    "transfers.gaps.list",
    "transfers.gaps.review",
    "transfers.rules.list",
    "transfers.rules.create",
    "transfers.rules.delete",
    "transfers.rules.enable",
    "transfers.rules.disable",
    "transfers.dismiss",
    "transfers.components.plan",
    "transfers.components.apply",
    "transfers.components.list",
    "transfers.components.show",
    "review.cases",
    "review.plan",
    "review.apply",
    "review.receipt",
    "reports.export-csv",
    "reports.export-xlsx",
    "reports.exit-tax",
    "reports.export-exit-tax-xlsx",
    "reports.export-austrian-e1kv-csv",
    "reports.export-austrian-e1kv-xlsx",
    "transactions.export",
    "wallets.import-btcpay",
    "wallets.import-ledger",
    "wallets.import-wasabi",
    "wallets.import-samourai",
    "reports.cross-check",
    "reports.export-year-handoff",
    "reports.privacy-hygiene",
    "source-funds.sources.list",
    "reports.source-funds",
    "source-funds.links.list",
    "source-funds.cases.list",
    "source-funds.recipients.list",
    "wallets.preview-document",
    "wallets.import-document",
    "source-funds.sources.create",
    "source-funds.sources.attach",
    "source-funds.recipients.create",
    "source-funds.recipients.update",
    "source-funds.recipients.delete",
    "source-funds.links.create",
    "source-funds.links.review",
    "source-funds.links.attach",
    "source-funds.links.bulk-review",
    "source-funds.review-context",
    "source-funds.request-input",
    "source-funds.coverage",
    "source-funds.suggest",
    "source-funds.assemble",
    "reports.lightning-profitability",
    "reports.export-lightning-profitability-csv",
    "reports.psbt-privacy",
    "chain-analysis.overview",
    "chain-analysis.trace",
    "chain-analysis.path",
    "chain-analysis.entropy",
    "chain-analysis.datasets.list",
    "chain-analysis.datasets.get",
    "chain-analysis.datasets.query",
    "chain-analysis.cases.list",
    "chain-analysis.cases.get",
    "chain-analysis.cases.compare",
    "chain-analysis.labels.list",
    "chain-analysis.psbt.analyze",
    "chain-analysis.psbt.compare",
    "chain-analysis.psbt.entropy",
    "chain-analysis.datasets.preview",
    "chain-analysis.labels.upsert",
    "chain-analysis.labels.import",
    "chain-analysis.labels.delete",
    "chain-analysis.cases.save",
    "chain-analysis.cases.delete",
    "chain-analysis.datasets.revoke",
    "chain-analysis.datasets.discard",
    "chain-analysis.watches.preview",
    "chain-analysis.watches.create",
    "chain-analysis.watches.list",
    "chain-analysis.watches.configure",
    "chain-analysis.watches.delete",
    "chain-analysis.watches.inbox",
    "chain-analysis.watches.acknowledge",
    "chain-analysis.watches.evaluate",
    "chain-analysis.sources.plan",
    "chain-analysis.sources.authorize",
    "chain-analysis.sources.list",
    "chain-analysis.sources.revoke",
    "chain-analysis.sources.run",
    "chain-analysis.acquire.plan",
    "chain-analysis.acquire.apply",
    "chain-analysis.datasets.import",
    "operator.unlock",
    "operator.status",
    "operator.lock",
    "operator.stop",
    "operator.mode",
    "operator.operation.status",
    "operator.operation.cancel",
    "mcp.tools",
    "mcp.status",
    "mcp.enable",
    "mcp.disable",
];

/// `_NO_BOOTSTRAP_DATABASE_PREFIXES`.
const NO_DATABASE_PREFIXES: &[&str] = &[
    "backup",
    "commands",
    "projects",
    "operator",
    "secrets",
    "offline",
    "update",
    "verify-download",
];

/// `_NO_BOOTSTRAP_DATABASE_PATHS`.
const NO_DATABASE_PATHS: &[&str] = &[
    "backends.kinds",
    "chat",
    "daemon",
    "mcp.serve",
    "mcp.tools",
    "mcp.status",
    "mcp.enable",
    "mcp.disable",
    "wallets.kinds",
    "wallets.ledger-template",
    "wallets.analyze-file",
];

/// `_SECRET_DEST_FRAGMENTS`.
const SECRET_FRAGMENTS: &[&str] = &[
    "payload",
    "api_key",
    "auth_header",
    "operator_auth",
    "passphrase",
    "password",
    "secret",
    "token",
];

/// One leaf of the tree: its path and parser, with the help its parent's
/// `add_parser(..., help=...)` gave it.
#[derive(Clone, Debug)]
pub struct Leaf {
    pub path: Vec<&'static str>,
    pub parser: ParserId,
    pub help: Option<&'static str>,
}

impl Leaf {
    pub fn dotted(&self) -> String {
        self.path.join(".")
    }

    pub fn implemented(&self) -> bool {
        IMPLEMENTED.contains(&self.dotted().as_str())
    }
}

/// `_walk_parser(parser)`: every leaf, aliases included, in argparse order.
pub fn leaves() -> Vec<Leaf> {
    fn walk(
        pid: ParserId,
        path: Vec<&'static str>,
        help: Option<&'static str>,
        out: &mut Vec<Leaf>,
    ) {
        let parser = &parsers()[pid];
        let Some(sub) = parser.subparsers() else {
            out.push(Leaf {
                path,
                parser: pid,
                help,
            });
            return;
        };
        for &(name, child) in sub.names {
            let help = sub
                .choice_help
                .iter()
                .find(|choice| choice.dest == name)
                .and_then(|choice| choice.help.text());
            let mut child_path = path.clone();
            child_path.push(name);
            walk(usize::from(child), child_path, help, out);
        }
    }
    let mut out = Vec::new();
    walk(0, Vec::new(), None, &mut out);
    out
}

/// `command_needs_database` for a leaf path, without the argument-dependent
/// exceptions (`wallets.import-ledger --dry-run`, `btcpay.key-url`).
pub fn needs_database(path: &str) -> bool {
    let top = path.split('.').next().unwrap_or("");
    !(NO_DATABASE_PREFIXES.contains(&top) || NO_DATABASE_PATHS.contains(&path))
}

fn effect(path: &str, capability: Option<Capability>) -> &'static str {
    if matches!(path, "chat" | "daemon" | "mcp.serve") {
        "interactive"
    } else if capability == Some(Capability::Read) {
        "read_only"
    } else {
        "mutating"
    }
}

fn opt_text(text: Option<&str>) -> Value {
    text.map_or(Value::Null, s)
}

/// One catalog entry.
fn entry(leaf: &Leaf) -> Value {
    let parser = &parsers()[leaf.parser];
    let dotted = leaf.dotted();
    let mut arguments = Vec::new();
    let mut destinations = Vec::new();
    let mut may_prompt = false;
    for action in parser.actions {
        if action.kind == ActionKind::Help {
            continue;
        }
        let dest = action.dest.unwrap_or("==SUPPRESS==");
        destinations.push(dest);
        let positional = action.is_positional();
        let required = if positional {
            !matches!(action.nargs, Nargs::Optional | Nargs::ZeroOrMore)
        } else {
            action.required
        };
        let secret_input = SECRET_FRAGMENTS.iter().any(|f| dest.contains(f));
        may_prompt |= secret_input;
        let choices = action.choices.map_or(Value::Null, |choices| {
            Value::List(choices.iter().map(|c| s(*c)).collect())
        });
        arguments.push(dict([
            ("name", s(dest)),
            (
                "flags",
                Value::List(action.option_strings.iter().map(|f| s(*f)).collect()),
            ),
            ("positional", Value::Bool(positional)),
            ("required", Value::Bool(required)),
            ("repeatable", Value::Bool(action.kind == ActionKind::Append)),
            ("choices", choices),
            ("secret_input", Value::Bool(secret_input)),
            (
                "help",
                match action.help {
                    Help::Text(text) => s(text),
                    Help::Missing | Help::Suppressed => Value::Null,
                },
            ),
        ]));
    }
    let capability = cli_capability(&dotted);
    let scope_flags: Vec<Value> = ["workspace", "profile", "wallet"]
        .into_iter()
        .filter(|name| destinations.contains(name))
        .map(s)
        .collect();
    dict([
        (
            "path",
            Value::List(leaf.path.iter().map(|p| s(*p)).collect()),
        ),
        ("command", s(leaf.path.join(" "))),
        ("kind", s(dotted.as_str())),
        (
            "help",
            opt_text(leaf.help.filter(|h| !h.is_empty()).or(parser.description)),
        ),
        ("effect", s(effect(&dotted, capability))),
        (
            "capability",
            capability.map_or(Value::Null, |c| s(c.as_str())),
        ),
        ("needs_database", Value::Bool(needs_database(&dotted))),
        (
            "supports_cursor",
            Value::Bool(destinations.contains(&"cursor")),
        ),
        (
            "supports_dry_run",
            Value::Bool(destinations.contains(&"dry_run")),
        ),
        ("scope_flags", Value::List(scope_flags)),
        ("may_prompt", Value::Bool(may_prompt)),
        ("arguments", Value::List(arguments)),
    ])
}

/// The catalog entry for one leaf, implemented or not (for drift tests).
pub fn describe_leaf(leaf: &Leaf) -> Value {
    entry(leaf)
}

/// `describe_command_catalog(parser, requested_path)`.
pub fn describe(query: &[String]) -> Result<Value, AppError> {
    let all = leaves();
    let matches = |leaf: &&Leaf| {
        leaf.path.len() >= query.len() && leaf.path.iter().zip(query).all(|(a, b)| *a == b)
    };
    let commands: Vec<Value> = all.iter().filter(matches).map(entry).collect();
    let query_value = Value::List(query.iter().map(|q| s(q.as_str())).collect());
    if !query.is_empty() && commands.is_empty() {
        return Err(AppError::new(
            "unknown_command",
            format!("unknown command path: {}", query.join(" ")),
        )
        .with_details(dict([("path", query_value)])));
    }
    Ok(dict([
        ("query", query_value),
        ("count", Value::from(commands.len())),
        (
            "global",
            dict([
                ("machine_implies_json", Value::Bool(true)),
                ("supports_non_interactive", Value::Bool(true)),
                ("secret_transport", s("file_descriptor_or_stdin_only")),
            ]),
        ),
        ("commands", Value::List(commands)),
    ]))
}
