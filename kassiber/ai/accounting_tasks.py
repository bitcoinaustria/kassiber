"""Small advertised task capability; no whole-book financial read tools."""


def catalog(entry):
    identifier = {'type': 'string', 'pattern': '^[a-f0-9]{32}$'}
    common = {'task_id': identifier}
    specifications = (
        ('task_get', 'read_only', common,
         'Read opaque progress of the task explicitly selected by the user through the accounting CLI. No financial records are disclosed. Read after interruption before any retry. Exceptions require explicit CLI input for the selected task; never claim completion from proposal text.'),
        ('task_preview', 'read_only', {**common, 'step': {'type': 'string', 'enum': ['prepare', 'post', 'close', 'tax_finalize', 'export_close', 'export_tax']}},
         'Prepare a fresh review handle for one supported task step. Only counts/status and a short-lived opaque handle reach you; exact financial consequences appear locally for user review. Preparation does not post; close and exports need their own approvals.'),
        ('task_apply', 'mutating', {**common, 'approval_id': {'type': 'string', 'minLength': 40, 'maxLength': 64},
                                   'idempotency_key': {'type': 'string', 'minLength': 1, 'maxLength': 128}},
         'Execute exactly the freshly previewed step after separate once-only user approval. Pass its unchanged approval_id. An expired or stale handle requires another preview. After interruption read task receipts; do not invent approval. Export prepares an artifact, NOT a saved file or filed return; explicit accounting CLI export output and package verification complete the local handoff.'),
        ('task_cancel', 'mutating', common,
         'Cancel the selected task after explicit user approval. This does not erase already committed accounting results or undo postings.'),
    )
    return tuple(entry(name='ui.accounting.'+name, wire_name='ui_accounting_'+name, daemon_kind='ui.accounting.'+name,
        kind_class=kind, description=description, summary_template='Accounting task: '+name,
        parameters={'type': 'object', 'additionalProperties': False, 'properties': properties, 'required': list(properties)})
        for name, kind, properties, description in specifications)
