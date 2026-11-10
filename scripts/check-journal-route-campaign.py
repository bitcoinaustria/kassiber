"""Resumable registered-route journal comparisons on disposable stored books.

Each generated or recorded operand becomes a real data root. Unchanged Python
(`tests.rust_port.journal_cli_reference`, with the registered tests' bound
clock and identities) and the test-harness binary then process copies of that
book through one registered route:

- `plain`: `kassiber --machine journals process`;
- `auto`: the same with `--auto-pair`;
- `desktop`: a daemon session sending `ui.journals.process`;
- `ai`: the assistant tool's journal step, Python's
  `_journals_refresh_payload(conn, auto_pair=False)` against the same native
  service reached through the test-only `test.journals.refresh` kind.

CLI routes compare exit status, stdout and stderr bytes. The desktop route
compares every row for the request, applying only the established
interpreter traceback allowance to `internal_error` debug strings. Every route
compares all application tables scalar-exactly, including REAL bits and rowids.

CLI routes run native with the development replay guard enabled. A guarded
refusal (exit 3) is accepted only when the native book is scalar-identical to
its starting state; it is counted by shape as the hybrid launcher's Python
replay, not as a native match. Python runs in a fork of a preloaded worker so
the campaign does not pay interpreter startup per operand.
"""
from __future__ import annotations

import argparse
from collections import Counter
import copy
import hashlib
import io
import json
import multiprocessing
import os
from pathlib import Path
import select
import shutil
import sqlite3
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

CLOCK = '2026-10-31T12:00:00Z'
KIND = 'ui.journals.process'
ROUTES = ('plain', 'auto', 'desktop', 'ai')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def census_corpus():
    """One operand per applicable perturbation of each census base book."""
    from tests.rust_port.journal_prebuild_campaign import materialize
    from tests.rust_port.journal_route_census import bases, perturbations
    corpus = []
    for suite, scenario, seed, generate in bases():
        snapshot = generate(seed)
        try:
            conn, _ = materialize(snapshot)
        except sqlite3.IntegrityError:
            conn, _ = materialize(snapshot, relaxed=True)
        try:
            names = [name for name, _ in perturbations(conn)]
        finally:
            conn.close()
        for name in names:
            corpus.append((dict(suite=suite, scenario=scenario, seed=seed, permuted=False, perturbation=name),
                           lambda g=generate, s=seed: g(s)))
    return corpus


def load_corpus(recordings, seeds):
    spec = __import__('importlib.util').util.spec_from_file_location(
        'process_campaign', ROOT / 'scripts/check-journal-process-campaign.py')
    module = __import__('importlib.util').util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return list(module.operands(recordings, seeds)), module


def fingerprint(binary, recordings):
    paths = {Path(__file__), ROOT / 'scripts/check-journal-process-campaign.py',
             *ROOT.glob('crates/**/*.rs'), *ROOT.glob('crates/**/Cargo.toml'),
             *ROOT.glob('kassiber/**/*.py'), *ROOT.glob('tests/custody*.py'),
             *ROOT.glob('tests/rust_port/journal*.py'), ROOT / 'Cargo.lock', ROOT / 'uv.lock'}
    if recordings:
        paths.update(recordings.rglob('*.json'))
    return dict(binary=digest(binary.read_bytes()), sources=digest(''.join(
        str(p.relative_to(ROOT) if p.is_relative_to(ROOT) else p) + ':' + digest(p.read_bytes()) + '\n'
        for p in sorted(paths)).encode()))


def environment(case):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('KASSIBER_OPERATOR_', 'KASSIBER_RUST_TEST_', 'KASSIBER_HYBRID_'))}
    owners = case / 'owners'
    owners.mkdir(exist_ok=True)
    env.update(
        KASSIBER_RUST_TEST_OWNER_ROOT=str(owners),
        KASSIBER_RUST_TEST_JOURNAL_TIME=CLOCK,
        KASSIBER_RUST_TEST_JOURNAL_IDS='sequential',
        KASSIBER_TEST_NO_EGRESS='1',
        KASSIBER_DISABLE_UPDATE_CHECK='1',
        XDG_DATA_HOME=str(case / 'xdg-data'),
        XDG_CONFIG_HOME=str(case / 'xdg-config'),
        XDG_STATE_HOME=str(case / 'xdg-state'),
    )
    # The Python reference children arm the test guard only through the
    # child hooks (tests/egress_guard.py); keep them first on the path.
    env['PYTHONPATH'] = os.pathsep.join(
        part for part in (str(ROOT / 'tests' / '_child_hooks'), str(ROOT), env.get('PYTHONPATH', '')) if part)
    return env


def template(directory):
    from kassiber.db import open_db
    root = directory / 'template'
    if not (root / 'kassiber.sqlite3').exists():
        open_db(root).close()
    return root


def build_book(snapshot, template_root, root, perturbation=None):
    """Copy the materialized operand into a freshly created, admitted book."""
    from tests.rust_port.journal_prebuild_campaign import materialize
    relaxed = False
    try:
        conn, omitted = materialize(snapshot)
    except sqlite3.IntegrityError:
        relaxed = True
        conn, omitted = materialize(snapshot, relaxed=True)
    source = root.parent / 'operand.sqlite3'
    try:
        target = sqlite3.connect(source)
        conn.backup(target)
        target.close()
    finally:
        conn.close()
    shutil.copytree(template_root, root)
    book = sqlite3.connect(root / 'kassiber.sqlite3', isolation_level=None)
    try:
        book.execute('PRAGMA foreign_keys=OFF')
        book.execute('ATTACH ? AS src', (str(source),))
        # materialize() inserted with CHECK constraints ignored; copy the same rows.
        book.execute('PRAGMA ignore_check_constraints=ON')
        book.execute('BEGIN')
        triggers = book.execute("SELECT name,sql FROM main.sqlite_master WHERE type='trigger'").fetchall()
        for name, _ in triggers:
            book.execute(f'DROP TRIGGER main."{name}"')
        if relaxed:
            for (name, sql) in book.execute("SELECT name,sql FROM src.sqlite_master WHERE type='table'").fetchall():
                live = book.execute("SELECT sql FROM main.sqlite_master WHERE type='table' AND name=?", (name,)).fetchone()
                if live and live[0] != sql:
                    book.execute(f'DROP TABLE main."{name}"')
                    book.execute(sql)
        for (name,) in book.execute("SELECT name FROM src.sqlite_master WHERE type='table' ORDER BY name").fetchall():
            # Keep the created book's own bookkeeping rows (settings, migration
            # audits, chain-index clock and dirty markers).
            if book.execute(f'SELECT 1 FROM main."{name}" LIMIT 1').fetchone():
                continue
            columns = [row[1] for row in book.execute(f'PRAGMA src.table_info("{name}")')]
            quoted = ','.join(f'"{c}"' for c in columns)
            book.execute(f'INSERT INTO main."{name}"({quoted}) SELECT {quoted} FROM src."{name}"')
        for _, sql in triggers:
            book.execute(sql)
        profile = snapshot['profile']
        book.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace',?),('context_profile',?)",
                     (profile['workspace_id'], profile['id']))
        book.execute('COMMIT')
        book.execute('PRAGMA ignore_check_constraints=OFF')
        book.execute('DETACH src')
    finally:
        book.close()
        source.unlink()
    # A user's book has already passed its compatibility migrations. Apply any
    # that the copied rows trigger once, before either runtime opens a copy,
    # so neither records a wall-clock migration audit of its own.
    from kassiber.db import open_db
    try:
        open_db(root).close()
    except Exception as error:  # relaxed operands may not be migratable
        if not relaxed:
            raise
        omitted = Counter(omitted, **{'adapter.migration_refused:' + type(error).__name__: 1})
    book = sqlite3.connect(root / 'kassiber.sqlite3', isolation_level=None)
    try:
        if perturbation is not None:
            from tests.rust_port.journal_route_census import perturbations
            statements = dict(perturbations(book)).get(perturbation)
            if statements is None:
                raise LookupError(f'census perturbation {perturbation} does not apply to the built book')
            book.execute('PRAGMA ignore_check_constraints=ON')
            book.execute('BEGIN')
            for sql, parameters in statements:
                book.execute(sql, parameters)
            book.execute('COMMIT')
            book.execute('PRAGMA ignore_check_constraints=OFF')
        book.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    finally:
        book.close()
    return relaxed, omitted


def book_dump(root):
    from tests.rust_port.journal_persistence_oracle import dump
    conn = sqlite3.connect(root / 'kassiber.sqlite3')
    # Census books may hold undecodable TEXT; keep its exact bytes comparable.
    conn.text_factory = lambda raw: raw.decode('utf-8', 'surrogateescape')
    try:
        integrity = conn.execute('PRAGMA integrity_check').fetchone()[0]
        return integrity, dump(conn)
    finally:
        conn.close()


def scalar_exact(a, b):
    from tests.test_rust_journal_process import scalar_exact as exact
    return exact(a, b)


def cli_arguments(root, route):
    return ['--machine', '--data-root', str(root), 'journals', 'process', *(('--auto-pair',) if route == 'auto' else ())]


def fork_python(env, argv, stdin_bytes, case, module='journal_cli_reference'):
    """Run unchanged Python in a fork of this preloaded worker."""
    stdin_path, stdout_path, stderr_path = case / 'py-stdin', case / 'py-stdout', case / 'py-stderr'
    stdin_path.write_bytes(stdin_bytes)
    pid = os.fork()
    if pid == 0:
        code = 70
        try:
            os.chdir(ROOT)
            os.environ.clear()
            os.environ.update(env)
            for fd, path, flags in ((0, stdin_path, os.O_RDONLY),
                                    (1, stdout_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC),
                                    (2, stderr_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC)):
                opened = os.open(path, flags, 0o600)
                os.dup2(opened, fd)
                os.close(opened)
            sys.stdin = io.TextIOWrapper(io.BufferedReader(io.FileIO(0, 'r', closefd=False)), encoding='utf-8')
            sys.stdout = io.TextIOWrapper(io.BufferedWriter(io.FileIO(1, 'w', closefd=False)), encoding='utf-8',
                                          line_buffering=True)
            sys.stderr = io.TextIOWrapper(io.BufferedWriter(io.FileIO(2, 'w', closefd=False)), encoding='utf-8',
                                          errors='backslashreplace', line_buffering=True)
            sys.argv = [module + '.py', *argv]
            reference = __import__('tests.rust_port.' + module, fromlist=['main'])
            try:
                result = reference.main()
                code = 0 if result is None else result
            except SystemExit as exit_:
                if exit_.code is None:
                    code = 0
                elif isinstance(exit_.code, int):
                    code = exit_.code
                else:
                    print(exit_.code, file=sys.stderr)
                    code = 1
            except BaseException:
                import traceback
                traceback.print_exc()
                code = 1
        finally:
            try:
                sys.stdout.flush()
                sys.stderr.flush()
            finally:
                os._exit(code if isinstance(code, int) and 0 <= code < 256 else 1)
    _, status = os.waitpid(pid, 0)
    return os.waitstatus_to_exitcode(status), stdout_path.read_bytes(), stderr_path.read_bytes()


def native_cli(binary, env, argv, guarded=True):
    env = dict(env)
    if guarded:
        env['KASSIBER_RUST_TEST_JOURNAL_FALLBACK'] = '1'
    completed = subprocess.run([str(binary), *argv], env=env,
                               capture_output=True, cwd=ROOT, timeout=300, check=False)
    return completed.returncode, completed.stdout, completed.stderr


def request_line(kind, request_id, args=None):
    row = {'kind': kind, 'request_id': request_id}
    if args is not None:
        row['args'] = args
    return (json.dumps(row) + '\n').encode()


def python_desktop(env, root, case):
    script = request_line('status', 'open') + request_line(KIND, 'process') + request_line('daemon.shutdown', 'stop')
    code, stdout, stderr = fork_python(env, ['--data-root', str(root), 'daemon'], script, case)
    rows = [json.loads(line) for line in stdout.splitlines() if line.strip()]
    return code, rows, stderr


def native_desktop(binary, env, root, request=None):
    process = subprocess.Popen([str(binary), '--data-root', str(root), 'daemon'], cwd=ROOT, env=env,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    buffer = b''
    rows = []

    def read(timeout=300):
        nonlocal buffer
        deadline = time.monotonic() + timeout
        while b'\n' not in buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
                raise TimeoutError('native daemon did not reply')
            data = os.read(process.stdout.fileno(), 65536)
            if not data:
                raise EOFError('native daemon closed output')
            buffer += data
        line, buffer = buffer.split(b'\n', 1)
        rows.append(json.loads(line))
        return rows[-1]

    def until(request_id, kind=None):
        while True:
            row = read()
            if row.get('request_id') == request_id and (row['kind'] == 'error' or kind is None or row['kind'] == kind):
                return row

    try:
        read()
        for line, request_id, kind in ((request_line('status', 'open'), 'open', None),
                                       (request or request_line(KIND, 'process'), 'process', KIND),
                                       (request_line('daemon.shutdown', 'stop'), 'stop', None)):
            process.stdin.write(line)
            process.stdin.flush()
            until(request_id, kind)
        process.stdin.close()
        process.wait(timeout=60)
        return process.returncode, rows, process.stderr.read()
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def wire_equal(actual, expected):
    """rust-shell.md's interpreter-specific traceback allowance, nothing else."""
    normalized = copy.deepcopy(expected)
    for row in normalized:
        error = row.get('error') if row.get('kind') == 'error' else None
        if error and error.get('code') == 'internal_error' and isinstance(error.get('debug'), str):
            if not error['debug'].startswith('Traceback (most recent call last):'):
                return False
            error['debug'] = None
    return json.dumps(actual) == json.dumps(normalized)


def compare(binary, route, snapshot, template_root, case, perturbation=None):
    python_root, native_root = case / 'python', case / 'rust'
    relaxed, omitted = build_book(snapshot, template_root, python_root, perturbation)
    shutil.copytree(python_root, native_root)
    integrity, before = book_dump(native_root)
    env = environment(case)
    row = dict(relaxed_schema=relaxed, omitted_columns=dict(omitted), fallback_shape=None)
    if route == 'ai':
        expected = fork_python(env, ['--data-root', str(python_root)], b'', case, module='journal_ai_reference')
        actual_code, actual_rows, actual_err = native_desktop(
            binary, env, native_root, request_line('test.journals.refresh', 'process', {'auto_pair': False}))
        actual = [r for r in actual_rows if r.get('request_id') == 'process']
        reply = json.loads(expected[1]) if expected[0] == 0 and expected[1] else {}
        terminal = actual[-1] if actual else {}
        if 'error' in reply:
            error = terminal.get('error') or {}
            same = terminal.get('kind') == 'error' and {k: error.get(k) for k in reply['error']} == reply['error']
        elif 'exception' in reply:
            same = terminal.get('kind') == 'error' and (terminal.get('error') or {}).get('code') == 'internal_error'
        else:
            same = terminal.get('kind') == KIND and json.dumps(terminal.get('data')) == json.dumps(reply.get('data'))
        row.update(refused='error' in reply or 'exception' in reply,
                   refusal_code=(reply.get('error') or {}).get('code') or reply.get('exception'))
        checks = dict(python_exit=expected[0] == 0 and expected[2] == b'', native_exit=actual_code == 0,
                      native_stderr=actual_err == b'', reply=same, single=len(actual) == 1)
        evidence = dict(expected=[expected[0], expected[1].decode('utf-8', 'replace'), expected[2].decode('utf-8', 'replace')],
                        actual=actual, actual_stderr=actual_err.decode('utf-8', 'replace'))
    elif route == 'desktop':
        expected_code, expected_rows, expected_err = python_desktop(env, python_root, case)
        actual_code, actual_rows, actual_err = native_desktop(binary, env, native_root)
        expected = [r for r in expected_rows if r.get('request_id') == 'process']
        actual = [r for r in actual_rows if r.get('request_id') == 'process']
        terminal = expected[-1] if expected else {}
        row.update(refused=terminal.get('kind') == 'error',
                   refusal_code=(terminal.get('error') or {}).get('code'),
                   python_traceback=expected_err.startswith(b'Traceback'))
        checks = dict(python_exit=expected_code == 0, native_exit=actual_code == 0,
                      native_stderr=actual_err == b'', wire=wire_equal(actual, expected),
                      terminal=bool(actual) and actual[-1].get('kind') in (KIND, 'error'))
        evidence = dict(expected=expected, actual=actual, expected_stderr=expected_err.decode('utf-8', 'replace'),
                        actual_stderr=actual_err.decode('utf-8', 'replace'))
    else:
        argv = cli_arguments(python_root, route)
        expected = fork_python(env, argv, b'', case)
        actual = native_cli(binary, env, cli_arguments(native_root, route))
        try:
            refusal = json.loads(expected[1]) if expected[0] else None
        except ValueError:
            refusal = None
        row.update(refused=expected[0] != 0,
                   refusal_code=((refusal or {}).get('error') or {}).get('code') if refusal else None,
                   python_traceback=b'Traceback' in expected[2])
        if actual[0] == 3:
            row['fallback_shape'] = actual[1].decode('ascii', 'replace').strip()
            _, after = book_dump(native_root)
            checks = dict(guard_signal=not actual[2] and bool(row['fallback_shape']),
                          rollback=scalar_exact(after, before))
            # Census only: would the unguarded native refusal have been exact?
            direct = native_cli(binary, env, cli_arguments(native_root, route), guarded=False)
            _, direct_after = book_dump(native_root)
            _, python_after = book_dump(python_root)
            row['native_equivalent'] = dict(
                exit=direct[0] == expected[0], stdout=direct[1] == expected[1], stderr=direct[2] == expected[2],
                dump=scalar_exact(direct_after, python_after))
            evidence = dict(expected=[expected[0], expected[1].decode('utf-8', 'replace'), expected[2].decode('utf-8', 'replace')],
                            actual=[actual[0], row['fallback_shape']],
                            direct=[direct[0], direct[1].decode('utf-8', 'replace'), direct[2].decode('utf-8', 'replace')])
            row.update(checks=checks, matched=all(checks.values()), integrity=integrity)
            if not all(row['native_equivalent'].values()):
                row['census_evidence'] = evidence
            return row, evidence
        checks = dict(exit=expected[0] == actual[0], stdout=expected[1] == actual[1], stderr=expected[2] == actual[2])
        evidence = dict(expected=[expected[0], expected[1].decode('utf-8', 'replace'), expected[2].decode('utf-8', 'replace')],
                        actual=[actual[0], actual[1].decode('utf-8', 'replace'), actual[2].decode('utf-8', 'replace')])
    native_code = None
    if route in ('desktop', 'ai'):
        native_code = ((actual[-1].get('error') or {}).get('code') if actual else None)
    elif actual[0] == 1:
        try:
            native_code = json.loads(actual[1])['error']['code']
        except (ValueError, KeyError, TypeError):
            pass
    if relaxed and native_code == 'requires_python_migration':
        # The adapter's relaxed tables are not an admitted current schema.
        # Native admission refuses before any write; the book must be intact.
        _, native_after = book_dump(native_root)
        checks = dict(schema_admission_unchanged=scalar_exact(native_after, before))
        row.update(checks=checks, matched=all(checks.values()), integrity=integrity, schema_admission=True)
        return row, evidence
    integrity_python, python_after = book_dump(python_root)
    integrity_native, native_after = book_dump(native_root)
    # Malformed operands may start with CHECK violations; both runtimes must
    # then leave the same report, and a sound book must stay sound.
    checks.update(integrity=integrity_python == integrity_native and (integrity != 'ok' or integrity_native == 'ok'),
                  dump=scalar_exact(native_after, python_after))
    row.update(checks=checks, matched=all(checks.values()), integrity=integrity,
               changed_tables=sum(before[t] != python_after.get(t) for t in before))
    return row, evidence


def write_private(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as output:
        json.dump(value, output, ensure_ascii=True, allow_nan=False, default=str)
        output.write('\n')


STATE = {}


def run_shard(shard):
    corpus, binary, route, destination, shard_size = (STATE[k] for k in ('corpus', 'binary', 'route', 'destination', 'shard_size'))
    scratch = destination / 'scratch' / f'{shard:04}'
    shutil.rmtree(scratch, ignore_errors=True)
    scratch.mkdir(parents=True)
    template_root = template(scratch)
    path = destination / f'{shard:04}.jsonl'
    rows = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
    start = shard * shard_size
    if [row['index'] for row in rows] != list(range(start, start + len(rows))):
        raise ValueError(f'incomplete or out-of-order shard log {path}')
    failures = 0
    try:
        for index in range(start + len(rows), min(len(corpus), start + shard_size)):
            metadata, generate = corpus[index]
            snapshot = generate()
            case = scratch / f'case-{index}'
            case.mkdir()
            started = time.monotonic()
            try:
                row, evidence = compare(binary, route, snapshot, template_root, case, metadata.get('perturbation'))
            except Exception as error:  # retain the operand; never skip silently
                row, evidence = dict(checks=dict(harness=False), matched=False, harness_error=repr(error)), dict(error=repr(error))
            row = dict(index=index, **metadata, route=route, seconds=round(time.monotonic() - started, 3),
                       input_sha256=digest(json.dumps(snapshot, sort_keys=True, ensure_ascii=True).encode()), **row)
            if not row['matched']:
                failures += 1
                write_private(destination / f'failure-{index:05}.json', dict(row=row, evidence=evidence))
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
            with os.fdopen(fd, 'a') as output:
                output.write(json.dumps(row, sort_keys=True) + '\n')
                output.flush()
                os.fsync(output.fileno())
            shutil.rmtree(case, ignore_errors=True)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    return shard, failures


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--route', choices=ROUTES, required=True)
    parser.add_argument('--recordings', type=Path)
    parser.add_argument('--census', action='store_true', help='perturbed census operands instead of the generated corpus')
    parser.add_argument('--seeds', type=int, default=2000)
    parser.add_argument('--shard-size', type=int, default=250)
    parser.add_argument('--jobs', type=int, default=4)
    parser.add_argument('--shards', type=int, nargs='*', help='only these shard numbers')
    args = parser.parse_args()
    destination = args.output.resolve()
    if not destination.is_relative_to((ROOT / 'target').resolve()):
        parser.error('campaign output must stay in the worktree target directory')
    if args.seeds < 1 or args.shard_size < 1 or args.jobs < 1:
        parser.error('positive sizes required')
    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    destination.chmod(0o700)
    if args.census and args.recordings:
        parser.error('--census builds its own operands')
    corpus = census_corpus() if args.census else load_corpus(args.recordings, args.seeds)[0]
    if args.recordings and len(corpus) != 318:
        parser.error(f'expected 159 retained recordings and permutations; found {len(corpus)} operands')
    if not args.recordings and not args.census and args.seeds == 2000 and len(corpus) != 25030:
        parser.error(f'expected 25,030 generated operands; found {len(corpus)}')
    binary = args.binary.resolve()
    manifest = dict(schema='kassiber.r4-route-campaign.v1', route=args.route, census=args.census,
                    **fingerprint(binary, args.recordings),
                    seeds=args.seeds, operands=len(corpus), shard_size=args.shard_size,
                    python_hash_seed=os.environ.get('PYTHONHASHSEED'),
                    adapter='Stored columns materialized into a freshly created book; omitted derived fields are counted. '
                            'Synthetic rate cache; no inferred ownership or legacy pair links.')
    manifest_path = destination / 'manifest.json'
    if manifest_path.exists():
        if json.loads(manifest_path.read_text()) != manifest:
            parser.error('campaign fingerprint changed; choose a new output directory')
    else:
        write_private(manifest_path, manifest)
    # Preload the reference so each forked operand skips interpreter startup.
    import kassiber.cli.main  # noqa: F401
    import kassiber.daemon  # noqa: F401
    import tests.rust_port.journal_cli_reference  # noqa: F401
    import tests.rust_port.journal_ai_reference  # noqa: F401
    import tests.rust_port.journal_prebuild_campaign  # noqa: F401
    import tests.test_rust_journal_process  # noqa: F401
    STATE.update(corpus=corpus, binary=binary, route=args.route, destination=destination, shard_size=args.shard_size)
    total = (len(corpus) + args.shard_size - 1) // args.shard_size
    shards = args.shards if args.shards else list(range(total))
    context = multiprocessing.get_context('fork')
    with context.Pool(args.jobs, maxtasksperchild=1) as pool:
        for shard, failures in pool.imap_unordered(run_shard, shards):
            print(json.dumps(dict(shard=shard, failures=failures)), flush=True)
    all_rows = [json.loads(line) for path in sorted(destination.glob('*.jsonl')) for line in path.read_text().splitlines()]
    counts = Counter(operands=len(all_rows), books=sum(not r['permuted'] for r in all_rows),
                     permutations=sum(r['permuted'] for r in all_rows),
                     native=sum(r['matched'] and not r.get('fallback_shape') and not r.get('schema_admission')
                                for r in all_rows),
                     replayed=sum(r['matched'] and bool(r.get('fallback_shape')) for r in all_rows),
                     failures=sum(not r['matched'] for r in all_rows),
                     python_refusals=sum(bool(r.get('refused')) for r in all_rows),
                     schema_admission=sum(bool(r.get('schema_admission')) for r in all_rows),
                     relaxed_schema=sum(bool(r.get('relaxed_schema')) for r in all_rows))
    shapes = Counter(r['fallback_shape'] for r in all_rows if r.get('fallback_shape'))
    refusal_codes = Counter(str(r.get('refusal_code')) for r in all_rows if r.get('refused'))
    census = {}
    for row in all_rows:
        if not row.get('perturbation'):
            continue
        kind = ':'.join(row['perturbation'].split(':')[::2]) if row['perturbation'].startswith('json:') else row['perturbation']
        outcome = ('failure' if not row['matched'] else 'replayed:' + row['fallback_shape'] if row.get('fallback_shape')
                   else 'native-refusal' if row.get('refused') else 'native-success')
        if row.get('fallback_shape'):
            outcome += ':equivalent' if all((row.get('native_equivalent') or {}).values()) else ':divergent'
        census.setdefault(kind, Counter())[outcome] += 1
    suites = {}
    for row in all_rows:
        suite = suites.setdefault(row['suite'], Counter())
        suite.update(operands=1, native=int(row['matched'] and not row.get('fallback_shape')),
                     replayed=int(bool(row.get('fallback_shape'))), failures=int(not row['matched']),
                     python_refusals=int(bool(row.get('refused'))))
    summary = dict(manifest=manifest, counts=counts, fallback_shapes=dict(shapes.most_common()),
                   refusal_codes=dict(refusal_codes.most_common()), suites=suites, census=census,
                   complete=len(all_rows) == len(corpus), failures=[r['index'] for r in all_rows if not r['matched']][:200])
    write_private(destination / 'summary.json', summary)
    print(json.dumps(dict(counts, complete=summary['complete'])), flush=True)
    return int(bool(counts['failures']))


if __name__ == '__main__':
    raise SystemExit(main())
