import argparse
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('document_cli_mutation_helpers', ROOT / 'tax-engine/scripts/mutation_check.py')
helper = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = helper
SPEC.loader.exec_module(helper)
CATALOG = Path(__file__).with_name('document_cli_mutations.json')


def pytest_outcome(result):
    summaries = re.findall(r'^(\d+ .+) in [\d.]+s(?: .*)?$', result.stdout, re.MULTILINE)
    if not summaries:
        return 2, False
    counts = {}
    for count, kind in re.findall(r'(\d+) (?:subtests )?(passed|failed|skipped|errors?)\b', summaries[-1]):
        counts[kind] = counts.get(kind, 0) + int(count)
    if (counts.get('skipped') or counts.get('error') or counts.get('errors')
            or not (counts.get('passed', 0) + counts.get('failed', 0))):
        return 2, False
    return result.returncode, bool(counts.get('failed') and 'AssertionError:' in result.stdout)


def problems():
    mutations = helper.load(CATALOG)
    errors = helper.problems(mutations)
    for mutation in mutations:
        for test in mutation.expect_failing:
            if test.startswith('tests/') and (error := helper._missing_test(ROOT, test)):
                errors.append(error)
    return errors


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--only', action='append', default=[])
    args = parser.parse_args()
    errors = problems()
    if errors:
        print('\n'.join(errors))
        return 2
    mutations = helper.load(CATALOG)
    if args.only:
        unknown = set(args.only) - {m.id for m in mutations}
        if unknown:
            parser.error(f'Unknown mutations: {sorted(unknown)}')
        mutations = [m for m in mutations if m.id in args.only]
    if args.check:
        print(f'{len(mutations)} document CLI entries apply')
        return 0
    sys.stdout.reconfigure(line_buffering=True)
    target = Path(os.environ['CARGO_TARGET_DIR']).resolve()
    if target == ROOT / 'target/r5-build' or not target.is_relative_to(ROOT / 'target'):
        parser.error('Use a separate disposable CARGO_TARGET_DIR')
    artifacts = ROOT / 'target/r5-document-artifacts'
    artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='document-cli-mutations-', dir=ROOT / 'target') as directory:
        scratch = Path(directory)
        helper.copy_worktree(ROOT, scratch)
        env = {**os.environ, 'PYTHONDONTWRITEBYTECODE': '1', 'CARGO_TARGET_DIR': str(target),
            'CARGO_BUILD_BUILD_DIR': str(target), 'CARGO_BUILD_JOBS': '8',
            'PYTHONPATH': str(scratch), 'KASSIBER_RUST_BIN': str(target / 'debug/kassiber')}
        env.pop('KASSIBER_DOCUMENT_CLI_COUNTS', None)
        cargo_env = helper._cargo_environment(env)
        helper._check_imports_resolve_to(scratch, env)
        def run(command, label, environment=env):
            result = subprocess.run(command, cwd=scratch, env=environment, capture_output=True, text=True, timeout=1800)
            (artifacts / (label + '.log')).write_text(result.stdout + result.stderr)
            return result
        def build(label):
            return run(['cargo', 'build', '-p', 'kassiber', '--features', 'test-harness', '--offline', '--locked'], label, cargo_env).returncode == 0
        def check(test, label):
            if test.startswith('tests/'):
                result = run([sys.executable, '-m', 'pytest', test, '-q', '-x', '--color=no'], label)
                return pytest_outcome(result)
            result = run(['cargo', 'test', '-p', 'kassiber-cli', '--lib', '--offline', '--locked', test], label, cargo_env)
            counts = [(int(a), int(b)) for a, b in helper._CARGO_RESULT.findall(result.stdout)]
            if not sum(a + b for a, b in counts):
                return 2, False
            return result.returncode, any(b for _, b in counts)
        if not build('mutation-baseline-build'):
            print('Baseline build failed')
            return 1
        for index, test in enumerate(sorted({test for m in mutations for test in m.expect_failing})):
            code, _ = check(test, f'mutation-baseline-{index}')
            if code:
                print(f'Baseline failed: {test}')
                return 1
        print('All named baselines pass')
        for mutation in mutations:
            file = scratch / mutation.file
            original = file.read_text()
            try:
                file.write_text(original.replace(mutation.find, mutation.replace, 1))
                if not build(mutation.id + '-build'):
                    print(f'{mutation.id}: build failed; no kill')
                    return 1
                killed = False
                for index, test in enumerate(mutation.expect_failing):
                    code, assertion_failed = check(test, mutation.id + f'-{index}')
                    if code == 1 or code == 101:
                        killed |= assertion_failed
                if not killed:
                    print(f'{mutation.id}: survived or no valid assertion failure')
                    return 1
                print(f'{mutation.id}: killed')
            finally:
                file.write_text(original)
        print(f'{len(mutations)} / {len(mutations)} document CLI mutations killed')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
