"""Python reference repro: truncated Electrum transaction bytes abort acquisition.

embit raises ``RuntimeError`` for truncated bytes. ``apply_acquisition`` only
records ``AppError``/``OSError``/``ValueError``/``TypeError`` as a frontier, so
one malformed response aborts the whole run instead of becoming a
``transaction_unavailable`` frontier. The native port records the frontier.
"""
from pathlib import Path
import sys
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def reproduce():
    from kassiber.core import chain_analysis_acquisition as acquisition

    class Budget:
        def request(self):
            return 1

    client = SimpleNamespace(socket=None, call=lambda method, params=None: 'deadbeef')
    reader = acquisition._Reader({'kind': 'electrum'}, Budget(), client)
    try:
        reader.transaction('ab' * 32, 'bitcoin')
    except (acquisition.AppError, OSError, ValueError, TypeError) as error:
        return {'caught_by_frontier': True, 'type': type(error).__name__}
    except Exception as error:
        return {'caught_by_frontier': False, 'type': type(error).__name__}
    return {'caught_by_frontier': None, 'type': None}


if __name__ == '__main__':
    print(reproduce())
