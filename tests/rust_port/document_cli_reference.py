from contextlib import ExitStack
import os
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest import mock
import uuid


def main():
    from kassiber.cli import handlers
    import importlib
    cli = importlib.import_module("kassiber.cli.main")
    from kassiber.operator import project
    from kassiber.core import imports, import_batches, attachments, metadata, document_import
    from kassiber.ai import providers

    project._owner_lock_root = lambda: Path(os.environ['KASSIBER_RUST_TEST_OWNER_ROOT'])
    timestamp = os.environ['KASSIBER_RUST_TEST_DOCUMENT_TIME']
    ids = iter(os.environ['KASSIBER_RUST_TEST_IMPORT_IDS'].split(','))
    evidence = iter(os.environ['KASSIBER_RUST_TEST_DOCUMENT_ATTACHMENT_IDS'].split(','))
    identities = SimpleNamespace(uuid4=lambda: uuid.UUID(next(ids)))
    attachment_ids = SimpleNamespace(uuid4=lambda: uuid.UUID(next(evidence)))
    with ExitStack() as stack:
        for module in [cli, handlers, imports, import_batches, attachments, metadata, document_import, providers]:
            if hasattr(module, 'now_iso'):
                stack.enter_context(mock.patch.object(module, 'now_iso', return_value=timestamp))
        for module in [handlers, imports, import_batches, metadata]:
            stack.enter_context(mock.patch.object(module, 'uuid', identities))
        stack.enter_context(mock.patch.object(attachments, 'uuid', attachment_ids))
        return cli.main(sys.argv[1:])


if __name__ == '__main__':
    raise SystemExit(main())
