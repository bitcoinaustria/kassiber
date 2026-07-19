"""Authoritative capability declarations for CLI commands and daemon kinds.

The operator broker uses these declarations as an authorization boundary.  The
tables are deliberately exact rather than suffix/prefix heuristics: a newly
added command or daemon kind is unclassified until a reviewer assigns it a
capability, and registry drift tests fail closed.
"""

from __future__ import annotations

from enum import Enum


class Capability(str, Enum):
    READ = "read"
    OPERATOR = "operator"
    ACCOUNTING_DECISIONS = "accounting_decisions"
    ADMIN = "admin"


CAPABILITY_ORDER = {
    Capability.READ: 0,
    Capability.OPERATOR: 1,
    Capability.ACCOUNTING_DECISIONS: 2,
    Capability.ADMIN: 3,
}

LEASE_CAPABILITIES = frozenset(
    {Capability.READ, Capability.OPERATOR, Capability.ACCOUNTING_DECISIONS}
)


def capability_allows(granted: Capability, required: Capability) -> bool:
    """Return whether a cumulative non-admin grant covers ``required``."""

    if required is Capability.ADMIN:
        return False
    if granted is Capability.ADMIN:
        return False
    return CAPABILITY_ORDER[granted] >= CAPABILITY_ORDER[required]


def _paths(value: str) -> frozenset[str]:
    return frozenset(part for part in value.split() if part)


CLI_READ_PATHS = _paths(
    """
    status health next-actions commands.describe projects.list projects.show
    chats.list chats.show secrets.status sync.status sync.transport.list
    sync.gc.status sync.members.list sync.devices.list sync.conflicts.list
    backends.list backends.kinds backends.get context.show context.current
    workspaces.list profiles.list profiles.get accounts.list wallets.list
    wallets.kinds wallets.get wallets.preview-document wallets.identify
    transactions.list attachments.list attachments.verify metadata.tags.list
    metadata.bip329.preview metadata.bip329.list metadata.records.list
    metadata.records.get metadata.records.history.list
    metadata.records.history.activity metadata.records.history.stale
    journals.list journals.quarantined journals.transfers.list
    journals.events.list journals.events.get journals.quarantine.show loans.list
    transfers.list transfers.gaps.list transfers.gaps.history transfers.gaps.plan
    transfers.components.list transfers.components.show transfers.components.plan
    transfers.payouts.list transfers.suggest transfers.rules.list views.list
    btcpay.provenance.list btcpay.provenance.links documents.list
    source-funds.sources.list source-funds.links.list source-funds.cases.list
    source-funds.coverage source-funds.recipients.list reports.summary
    reports.tax-summary reports.balance-sheet reports.portfolio-summary
    reports.capital-gains reports.journal-entries reports.privacy-hygiene
    reports.privacy-mirror reports.psbt-privacy reports.austrian-e1kv
    reports.austrian-tax-summary reports.exit-tax reports.filed-snapshots.list
    reports.balance-history reports.lightning-profitability
    reports.commercial-subledger reports.source-funds rates.pairs rates.latest
    rates.range ai.providers.list ai.providers.get ai.models
    """
)

CLI_OPERATOR_PATHS = _paths(
    """
    projects.select chat backends.create backends.update backends.set-default
    backends.clear-default context.set workspaces.create profiles.create
    profiles.set accounts.create wallets.create wallets.update
    wallets.import-json wallets.import-csv wallets.import-btcpay
    wallets.import-phoenix wallets.import-wasabi wallets.import-river
    wallets.import-bull wallets.import-bullbitcoin wallets.import-coinfinity
    wallets.import-21bitcoin wallets.import-pocket wallets.import-pocketbitcoin
    wallets.import-strike wallets.import-ledger-live
    wallets.import-binance-supplemental wallets.sync-kraken
    wallets.sync-coinbase wallets.sync-binance wallets.import-ledger
    wallets.ledger-template wallets.import-document wallets.import-samourai
    wallets.sync-btcpay wallets.attach-btcpay wallets.attach-bullbitcoin-wallet
    wallets.sync wallets.derive transactions.export attachments.add
    attachments.rename attachments.remove attachments.gc metadata.notes.set
    metadata.notes.clear metadata.tags.create metadata.tags.add
    metadata.tags.remove metadata.bip329.import metadata.bip329.export
    metadata.records.note.set metadata.records.note.clear
    metadata.records.tag.add metadata.records.tag.remove journals.process
    btcpay.provenance.sync btcpay.provenance.suggest documents.create
    documents.attach source-funds.sources.create source-funds.sources.attach
    source-funds.links.create source-funds.links.attach source-funds.suggest
    source-funds.assemble source-funds.recipients.create
    source-funds.recipients.update source-funds.recipients.delete
    reports.export-pdf reports.export-summary-pdf reports.export-csv
    reports.export-xlsx reports.export-lightning-profitability-csv
    reports.export-commercial-subledger-csv reports.export-source-funds-pdf
    reports.export-source-funds-bundle reports.export-austrian-e1kv-pdf
    reports.export-austrian reports.export-austrian-e1kv-xlsx
    reports.export-austrian-e1kv-csv reports.export-exit-tax-pdf
    reports.export-exit-tax-xlsx rates.sync rates.rebuild rates.set
    diagnostics.collect
    """
)

CLI_ACCOUNTING_DECISION_PATHS = _paths(
    """
    metadata.exclude metadata.include metadata.records.excluded.set
    metadata.records.excluded.clear metadata.records.history.revert
    journals.quarantine.clear journals.quarantine.resolve.price-override
    journals.quarantine.resolve.exclude loans.mark loans.unmark loans.link
    transfers.pair transfers.unpair transfers.update transfers.gaps.review
    transfers.gaps.apply transfers.components.apply transfers.payouts.create
    transfers.payouts.delete transfers.bulk-pair transfers.dismiss
    transfers.rules.create transfers.rules.delete transfers.rules.enable
    transfers.rules.disable transfers.rules.apply views.create views.delete
    btcpay.provenance.review source-funds.links.review
    source-funds.links.bulk-review reports.filed-snapshots.create
    """
)

CLI_ADMIN_PATHS = _paths(
    """
    daemon init projects.create chats.delete chats.clear chats.config
    secrets.init secrets.init-resume secrets.change-passphrase
    secrets.remember-unlock secrets.forget-unlock secrets.verify
    secrets.migrate-credentials backup.export backup.import sync.enable
    sync.disable sync.transport.add sync.transport.remove sync.lan.listen
    sync.lan.connect sync.lan.discover sync.tor.listen sync.tor.connect
    sync.gc.run sync.join-request sync.invite sync.join sync.push sync.pull
    sync.members.revoke sync.devices.revoke sync.conflicts.resolve
    backends.delete backends.reveal-token wallets.delete
    wallets.reveal-descriptor ai.providers.create ai.providers.update
    ai.providers.delete ai.providers.set-default ai.providers.clear-default
    """
)


DAEMON_READ_KINDS = _paths(
    """
    status ui.logs.snapshot ui.egress.snapshot ui.overview.snapshot
    ui.workspace.overview.snapshot ui.transactions.list
    ui.transactions.dashboard ui.transactions.extremes ui.transactions.resolve
    ui.transactions.graph ui.transactions.review_context
    ui.transactions.search ui.transactions.ledger_template
    ui.transactions.history ui.activity.history ui.activity.stale
    ui.attachments.list ui.attachments.open ui.wallets.list ui.wallets.utxos
    ui.privacy_hygiene.snapshot ui.wallets.identify ui.loans.list
    ui.backends.list ui.backends.options ui.backends.public_defaults
    ui.backends.settings.list ui.reports.capital_gains ui.reports.summary
    ui.reports.balance_sheet ui.reports.portfolio_summary
    ui.reports.tax_summary ui.reports.balance_history
    ui.reports.privacy_hygiene ui.reports.privacy_mirror
    ui.reports.psbt_privacy ui.reports.exit_tax_preview
    ui.source_funds.preview ui.source_funds.cases.list
    ui.source_funds.sources.list ui.source_funds.links.list
    ui.source_funds.evidence.list ui.source_funds.coverage
    ui.source_funds.recipients.list ui.btcpay.provenance.list
    ui.btcpay.provenance.links ui.transactions.commercial_context
    ui.documents.list ui.journals.snapshot ui.journals.events.list
    ui.journals.quarantine ui.journals.transfers.list ui.transfers.suggest
    ui.transfers.review_context ui.transfers.list ui.transfers.payouts.list
    ui.transfers.components.list ui.transfers.components.get
    ui.transfers.components.plan ui.custody.coverage.snapshot
    ui.custody.lineage.snapshot ui.custody.gaps.list
    ui.custody.gaps.review_context ui.custody.gaps.history
    ui.custody.review.plan ui.transfers.rules.list ui.saved_views.list
    ui.profiles.snapshot ui.rates.summary ui.rates.coverage ui.rates.latest
    ui.report.blockers ui.audit.changes_since_last_answer
    ui.audit.evidence.summary ui.review.worklist ui.maintenance.settings
    ui.sync.status ui.sync.transports.list ui.sync.members.list
    ui.sync.devices.list ui.sync.conflicts.list ui.freshness.status
    ui.workspace.health ui.projects.list ui.next_actions ui.review.badges
    ui.wallets.ledger_preview ui.wallets.preview_descriptor
    ui.wallets.detect_script_types ui.connections.sources
    ui.connections.node.snapshot ui.reports.lightning_profitability
    ui.metadata.bip329.preview ai.providers.list ai.providers.get
    ai.list_models ui.chat.sessions.list ui.chat.sessions.get
    """
)

DAEMON_OPERATOR_KINDS = _paths(
    """
    ui.transactions.export_csv ui.transactions.export_xlsx
    ui.transactions.metadata.update ui.attachments.add ui.attachments.copy
    ui.attachments.rename ui.attachments.remove ui.wallets.identify_onchain
    ui.backends.create ui.backends.update ui.backends.set_default
    ui.backends.bitcoinrpc.test ui.backends.btcpay.test
    ui.backends.detect_core ui.backends.electrum.test ui.backends.http.test
    ui.backends.lightning.test ui.reports.export_pdf
    ui.reports.export_summary_pdf ui.reports.export_csv ui.reports.export_xlsx
    ui.reports.export_capital_gains_csv ui.reports.export_austrian_e1kv_pdf
    ui.reports.export_austrian_e1kv_xlsx ui.reports.export_austrian_e1kv_csv
    ui.reports.export_exit_tax_pdf ui.reports.export_exit_tax_xlsx
    ui.reports.export_audit_package ui.source_funds.sources.create
    ui.source_funds.sources.attach ui.source_funds.links.create
    ui.source_funds.links.attach ui.source_funds.suggest
    ui.source_funds.assemble ui.source_funds.export_pdf
    ui.source_funds.export_bundle ui.source_funds.recipients.create
    ui.source_funds.recipients.update ui.source_funds.recipients.delete
    ui.btcpay.provenance.sync ui.btcpay.provenance.suggest
    ui.documents.create ui.documents.attach ui.journals.process
    ui.saved_views.create ui.saved_views.delete ui.onboarding.complete
    ui.profiles.create ui.profiles.rename ui.profiles.update
    ui.profiles.switch ui.rates.kraken_csv.import ui.rates.rebuild
    ui.maintenance.configure ui.maintenance.run ui.freshness.configure
    ui.freshness.run ui.freshness.cancel ui.freshness.pause
    ui.freshness.resume ui.workspace.freshness.run ui.workspace.create
    ui.workspace.rename ui.wallets.create ui.wallets.import_file
    internal.document_import.stage ui.wallets.document_import.preview
    ui.wallets.document_import.import ui.wallets.import_samourai
    ui.connections.btcpay.create ui.connections.bullbitcoin_wallet.create
    ui.connections.btcpay.discover ui.connections.btcpay.test
    ui.metadata.bip329.import ui.metadata.bip329.export ui.wallets.update
    ui.wallets.sync ai.chat ai.chat.cancel ai.tool_call.consent
    """
)

DAEMON_ACCOUNTING_DECISION_KINDS = _paths(
    """
    ui.transactions.history.revert ui.loans.link ui.loans.mark ui.loans.unmark
    ui.source_funds.cases.save ui.source_funds.links.review
    ui.source_funds.links.bulk_review ui.btcpay.provenance.review
    ui.journals.quarantine.resolve ui.transfers.payouts.create
    ui.transfers.payouts.delete ui.transfers.pair ui.transfers.unpair
    ui.transfers.update ui.transfers.bulk_pair ui.transfers.dismiss
    ui.transfers.components.apply ui.custody.review.apply ui.transfers.rules.create
    ui.transfers.rules.delete ui.transfers.rules.set_enabled
    ui.transfers.rules.apply
    """
)

DAEMON_ADMIN_KINDS = _paths(
    """
    ui.backends.delete ui.sync.enable ui.sync.disable
    ui.sync.transports.configure ui.sync.transports.delete ui.sync.push
    ui.sync.pull ui.sync.join_request ui.sync.invite ui.sync.join
    ui.sync.members.revoke ui.sync.devices.revoke ui.sync.conflicts.resolve
    ui.workspace.delete ui.profiles.reset_data ui.projects.create
    ui.projects.select ui.secrets.init ui.secrets.change_passphrase
    ui.secrets.forget_cli_unlock ui.wallets.delete daemon.lock daemon.unlock
    ai.providers.create ai.providers.update ai.providers.set_api_key
    ai.providers.move_api_key ai.providers.delete ai.providers.set_default
    ai.providers.clear_default ai.providers.acknowledge ai.test_connection
    ui.chat.sessions.delete ui.chat.sessions.clear ui.chat.history.configure
    wallets.reveal_descriptor backends.reveal_token daemon.shutdown
    """
)


def _build_registry(
    groups: tuple[tuple[Capability, frozenset[str]], ...],
) -> dict[str, Capability]:
    registry: dict[str, Capability] = {}
    for capability, paths in groups:
        for path in paths:
            if path in registry:
                raise RuntimeError(f"duplicate capability declaration for {path!r}")
            registry[path] = capability
    return registry


CLI_CAPABILITIES = _build_registry(
    (
        (Capability.READ, CLI_READ_PATHS),
        (Capability.OPERATOR, CLI_OPERATOR_PATHS),
        (Capability.ACCOUNTING_DECISIONS, CLI_ACCOUNTING_DECISION_PATHS),
        (Capability.ADMIN, CLI_ADMIN_PATHS),
    )
)

DAEMON_CAPABILITIES = _build_registry(
    (
        (Capability.READ, DAEMON_READ_KINDS),
        (Capability.OPERATOR, DAEMON_OPERATOR_KINDS),
        (Capability.ACCOUNTING_DECISIONS, DAEMON_ACCOUNTING_DECISION_KINDS),
        (Capability.ADMIN, DAEMON_ADMIN_KINDS),
    )
)


def cli_capability(path: str) -> Capability:
    try:
        return CLI_CAPABILITIES[path]
    except KeyError:
        raise KeyError(f"unclassified CLI command {path!r}") from None


def daemon_capability(kind: str) -> Capability:
    try:
        return DAEMON_CAPABILITIES[kind]
    except KeyError:
        raise KeyError(f"unclassified daemon kind {kind!r}") from None
