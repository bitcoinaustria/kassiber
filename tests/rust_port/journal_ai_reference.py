"""Disposable reference for the assistant's scoped journal step.

`ai.chat` runs the `ui.journals.process` tool as
`_journals_refresh_payload(conn, auto_pair=False)` on the daemon's main
connection. This runs that exact step on a disposable book under the clock and
identity bindings of `journal_cli_reference`, and prints one JSON line: the
`ui.journals.process` envelope, or the refusal the tool would report.

    python -m tests.rust_port.journal_ai_reference --data-root <root>
"""
import argparse
import json
import os
from pathlib import Path
import sys
from unittest.mock import patch


def step(argv):
    from kassiber.daemon import _journals_refresh_payload
    from kassiber.db import open_db
    from kassiber.envelope import build_envelope
    from kassiber.errors import AppError
    parser = argparse.ArgumentParser()
    parser.add_argument('--data-root', type=Path, required=True)
    args = parser.parse_args(argv)
    passphrase = os.environ.get('KASSIBER_TEST_JOURNAL_AI_PASSPHRASE')
    conn = None
    try:
        # The daemon's book open precedes the tool; its refusal is the reply.
        conn = open_db(args.data_root, passphrase=passphrase)
        value = build_envelope('ui.journals.process', _journals_refresh_payload(conn, auto_pair=False))
    except AppError as error:
        value = dict(error=dict(code=error.code, message=str(error), hint=error.hint,
                                details=error.details, retryable=error.retryable))
    except Exception as error:  # the chat loop reports these as internal tool failures
        value = dict(exception=type(error).__name__, message=str(error))
    finally:
        if conn is not None:
            conn.close()
    sys.stdout.write(json.dumps(value) + '\n')
    sys.stdout.flush()
    return 0


def main():
    # Reuse the CLI reference's bindings verbatim; it imports the CLI entry
    # point at call time, so substitute the scoped step for it.
    from tests.rust_port import journal_cli_reference
    with patch('kassiber.cli.main.main', step):
        return journal_cli_reference.main()


if __name__ == '__main__':
    raise SystemExit(main())
