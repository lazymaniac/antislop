#!/usr/bin/env python3
"""Run a real, scoped antislop task against an isolated copy of its own source."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]


def fingerprints(workspace):
    return {str(p.relative_to(workspace)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in workspace.rglob('*') if p.is_file()
            and 'target' not in p.relative_to(workspace).parts
            and '__pycache__' not in p.relative_to(workspace).parts}


def verified_sessions(run):
    ledger = run.get('session_ledger', {})
    sessions = ledger.get('sessions', {})
    turns = ledger.get('turns', {})
    artifacts = {artifact['hash']: artifact for artifact in run['artifacts']}
    observed = {}
    pairs = []
    for review in run['reviews']:
        if artifacts.get(review['artifact_hash'], {}).get('session_id') != review['executor_session']:
            return []
        native = []
        for field, role in (('executor_session', 'executor'), ('verifier_session', 'verifier')):
            session = sessions.get(review[field], {})
            turn = turns.get(session.get('current_turn_id'), {})
            if (turn.get('session_id') != review[field] or not session.get('native_observed') or session.get('state') != 'completed'
                    or session.get('scope', {}).get('role') != role
                    or turn.get('state') != 'completed' or not turn.get('output_ref')
                    or hashlib.sha256(turn.get('output', '').encode()).hexdigest() != turn['output_ref']['hash']):
                return []
            try:
                identity = (session['scope']['provider'], str(uuid.UUID(session['native_session_id'])))
            except (KeyError, ValueError, TypeError):
                return []
            if observed.setdefault(identity, review[field]) != review[field]:
                return []
            native.append(identity)
        if native[0] == native[1]:
            return []
        pairs.append(dict(executor=native[0], verifier=native[1]))
    return pairs


def main():
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--live', action='store_true', required=True,
                        help='Authorize authenticated Codex usage and checks in a disposable source copy')
    parser.add_argument('--repairs', type=int, default=2)
    args = parser.parse_args()
    if args.repairs < 0:
        parser.error('--repairs must be nonnegative')
    binary = ROOT / 'target/debug/antislop'
    if not binary.exists():
        raise SystemExit('Build first: scripts/cargo.sh build --locked')
    root = Path(tempfile.mkdtemp(prefix='antislop-selfhost-', dir='/tmp'))
    root.chmod(0o700)
    workspace = root / 'source'
    workspace.mkdir()
    # Source only: no user state, Git metadata, credentials or build products.
    for name in ('src', 'tests', 'scripts', 'fixtures', 'examples', 'docs', '.github'):
        source = ROOT / name
        if source.exists():
            shutil.copytree(source, workspace / name, ignore=shutil.ignore_patterns('__pycache__'))
    for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'README.md', 'design.md', 'LICENSE'):
        if (ROOT / name).exists():
            shutil.copy2(ROOT / name, workspace / name)
    check = workspace / 'selfhost_check.py'
    check.write_text('''import pathlib, subprocess, tomllib
root = pathlib.Path(__file__).resolve().parent
subprocess.run([str(root / "scripts/cargo.sh"), "clippy", "--all-targets", "--locked", "--", "-D", "warnings"], cwd=root, check=True)
subprocess.run([str(root / "scripts/cargo.sh"), "test", "--all-targets", "--locked"], cwd=root, check=True)
subprocess.run([str(root / "scripts/cargo.sh"), "build", "--locked"], cwd=root, check=True)
result = subprocess.run([str(root / "target/debug/antislop"), "--version"], cwd=root, capture_output=True, text=True)
if result.returncode != 0: raise SystemExit(result.stderr)
version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
if result.stdout != f"antislop {version}\\n": raise SystemExit(repr(result.stdout))
if result.stderr != "": raise SystemExit(repr(result.stderr))
help_result = subprocess.run([str(root / "target/debug/antislop"), "--help"], cwd=root, capture_output=True, text=True)
if help_result.returncode != 0 or "antislop" not in help_result.stdout: raise SystemExit("help regression")
''')
    version = tomllib.loads((workspace / 'Cargo.toml').read_text())['package']['version']
    clippy_version = subprocess.check_output([workspace / 'scripts/cargo.sh', 'clippy', '--version'], cwd=workspace, text=True).strip()
    (workspace / 'selfhost_environment.json').write_text(json.dumps(dict(analyzer='clippy', version=clippy_version, analyzer_command=['scripts/cargo.sh', 'clippy', '--all-targets', '--locked', '--', '-D', 'warnings'], configuration='dev', target='host', dependency_scope='No dependencies added, removed, or changed; no distribution requested. Existing dependency audit is outside this change.'), indent=2) + '\n')
    with (root / 'preflight-clippy.log').open('w') as log:
        preflight = subprocess.run([workspace / 'scripts/cargo.sh', 'clippy', '--all-targets', '--locked', '--', '-D', 'warnings'], cwd=workspace, stdout=log, stderr=log)
    if preflight.returncode:
        raise SystemExit(f'Existing analyzer failure; resolve before provider usage: {root / "preflight-clippy.log"}')
    subprocess.run([workspace / 'scripts/cargo.sh', 'build', '--locked'], cwd=workspace, check=True)
    baseline = subprocess.run([workspace / 'target/debug/antislop', '--version'], capture_output=True, text=True)
    if baseline.returncode == 0:
        raise SystemExit('Task already implemented; choose a new self-host task')
    (root / 'baseline.json').write_text(json.dumps(dict(exit_code=baseline.returncode, stdout=baseline.stdout, stderr=baseline.stderr), indent=2) + '\n')
    spec = dict(objective='Add --version to antislop itself. It must print exactly antislop followed by a space and the Cargo package version and a newline, exit successfully, and write nothing to stderr. Read the version using env!("CARGO_PKG_VERSION") to avoid duplicating it. Preserve existing commands. Change only src/main.rs. The immutable selfhost_check.py runs the full Rust suite, builds, and verifies CLI behavior.',
                workspace=str(workspace), provider='codex', model=None,
                requirements=[dict(id='REQ-VERSION', original_text='antislop --version reports the Cargo package version successfully; existing commands keep working.', source_ref='selfhost:version', criteria=dict(version=f'--version exits 0 with stdout antislop {version} newline and no stderr, derived from CARGO_PKG_VERSION', regression='The full Rust test suite and --help still pass'))],
                plan_items=[dict(id='PLAN-VERSION', requirement_ids=['REQ-VERSION'], description='Inspect CLI dispatch, compare version handling options, implement version before command argument parsing, independently verify exact output and regressions.')],
                input_files=['src/main.rs', 'Cargo.toml', 'selfhost_check.py', 'selfhost_environment.json'], editable_files=['src/main.rs'],
                check_command=[shutil.which('python3'), str(check)])
    (root / 'spec.json').write_text(json.dumps(spec, indent=2) + '\n')
    before = (workspace / 'src/main.rs').read_bytes()
    source_hashes = fingerprints(workspace)
    (root / 'source-hashes.json').write_text(json.dumps(source_hashes, indent=2) + '\n')
    state = root / 'state'
    state.mkdir(mode=0o700)
    print(f'Preserved self-host workspace and evidence: {root}', flush=True)
    with (root / 'daemon.log').open('w') as daemon_log, (root / 'driver.log').open('w') as driver_log:
        daemon = subprocess.Popen([binary, 'daemon', state], stdout=daemon_log, stderr=daemon_log)
        try:
            for _ in range(100):
                if daemon.poll() is not None:
                    raise RuntimeError('Daemon failed; inspect daemon.log')
                if (state / 'architect.sock').exists():
                    break
                time.sleep(.05)
            result = subprocess.run([binary, 'run', state, root / 'spec.json', '--approve-scoped', str(args.repairs)], stdout=driver_log, stderr=driver_log)
            listing = subprocess.run([binary, 'list', state], capture_output=True, text=True, check=True)
            runs = json.loads(listing.stdout)['data']['runs']
            run_id = runs[0]['id']
            exported = subprocess.run([binary, 'export', state, run_id], capture_output=True, check=True)
            (root / 'run.json').write_bytes(exported.stdout)
            run = json.loads(exported.stdout)
            pairs = verified_sessions(run)
            after = (workspace / 'src/main.rs').read_bytes()
            report = dict(schema_version=1, task='antislop adds its own --version', provider='codex',
                          isolated_workspace=str(workspace), run_id=run_id, status=run['status'],
                          driver_exit_code=result.returncode, source_changed=before != after,
                          elapsed_seconds=round(time.monotonic() - started, 2),
                          before_hash=hashlib.sha256(before).hexdigest(), after_hash=hashlib.sha256(after).hexdigest(),
                          artifacts=len(run['artifacts']), reviews=len(run['reviews']),
                          attempts=run['attempt'],
                          repair_approvals=sum(a['action'] == 'repair' for a in run['approvals']),
                          turn_usage={turn_id: turn['usage'] for turn_id, turn in run.get('session_ledger', {}).get('turns', {}).items()},
                          independent_sessions=bool(pairs) and len(pairs) == len(run['reviews']),
                          native_session_pairs=pairs,
                          real_quality=bool(run['reviews']) and all(r.get('quality') and not r['quality']['synthetic'] for r in run['reviews']),
                          diagnostic=run.get('diagnostic'))
            (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            print(json.dumps(report, indent=2), flush=True)
            if result.returncode or run['status'] != 'completed' or before == after or not report['independent_sessions'] or not report['real_quality']:
                raise SystemExit(1)
            if b'env!("CARGO_PKG_VERSION")' not in after:
                raise SystemExit('Version derivation requirement missing from generated source')
            final = subprocess.run(spec['check_command'], cwd=workspace, capture_output=True)
            (root / 'final-check.log').write_bytes(final.stdout + final.stderr)
            if final.returncode:
                raise SystemExit('Post-completion acceptance failed; inspect final-check.log')
            final_hashes = fingerprints(workspace)
            changed = {p for p in source_hashes.keys() | final_hashes.keys()
                       if source_hashes.get(p) != final_hashes.get(p)}
            if changed != {'src/main.rs'}:
                raise SystemExit(f'Unexpected source changes: {sorted(changed)}')
            report['final_acceptance_passed'] = True
            report['changed_files'] = sorted(changed)
            report['baseline_version_exit_code'] = baseline.returncode
            (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        finally:
            daemon.terminate()
            daemon.wait(timeout=15)


if __name__ == '__main__':
    main()
