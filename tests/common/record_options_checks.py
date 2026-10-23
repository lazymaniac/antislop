"""Record command output verbatim with source hashes; no external attestation."""
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / '.antislop/options-evidence'
BASE = '3f77db2'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def inputs(root):
    paths = [p for folder in ['src', 'tests', 'examples']
             for p in (root / folder).rglob('*')
             if p.is_file() and 'options-evidence' not in p.parts
             and p.suffix not in ['.md', '.log']]
    paths += [root / name for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']]
    return {str(p.relative_to(root)): digest(p.read_bytes()) for p in sorted(paths)}


def execute(label, root, command, env):
    before = inputs(root)
    start = time.time()
    result = subprocess.run(command, cwd=root, env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, check=False)
    log = OUTPUT / (label + '.log')
    log.write_bytes(result.stdout)
    record = {'command': command, 'cwd': str(root), 'started_unix': start,
              'duration_seconds': time.time() - start, 'exit_code': result.returncode,
              'log_sha256': digest(result.stdout), 'inputs_before': before,
              'inputs_after': inputs(root)}
    (OUTPUT / (label + '.json')).write_text(json.dumps(record, indent=2) + '\n')
    print(label, result.returncode, flush=True)


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    metadata = {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip(),
                'base': subprocess.check_output(['git', 'rev-parse', BASE], cwd=ROOT).decode().strip(),
                'diff': subprocess.check_output(['git', 'diff', '--', 'tests/lean_options.rs', 'tests/common'], cwd=ROOT).decode(),
                'platform': platform.platform(),
                'rustc': subprocess.check_output(['rustc', '-Vv'], cwd=ROOT).decode(),
                'cargo': subprocess.check_output(['cargo', '-V'], cwd=ROOT).decode(),
                'runner_sha256': digest(Path(__file__).read_bytes()),
                'environment': {k: os.environ[k] for k in ['PATH', 'TMPDIR', 'RUSTUP_TOOLCHAIN', 'CARGO_TARGET_DIR'] if k in os.environ}}
    (OUTPUT / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    env = dict(os.environ)
    env['CARGO_TARGET_DIR'] = str(ROOT / 'target')
    with tempfile.TemporaryDirectory(prefix='antislop-options-base-') as temporary:
        archive = subprocess.check_output(['git', 'archive', BASE], cwd=ROOT)
        with tarfile.open(fileobj=io.BytesIO(archive)) as source:
            source.extractall(temporary, filter='data')
        for name in ['lean_core', 'lean_tui']:
            execute('base-' + name, Path(temporary), ['cargo', 'test', '--test', name], env)
    for name in ['lean_options', 'lean_core', 'lean_tui']:
        execute('head-' + name, ROOT, ['cargo', 'test', '--test', name], env)
    for name, command in [('fmt', ['cargo', 'fmt', '--check']),
                          ('check', ['cargo', 'check']),
                          ('clippy', ['cargo', 'clippy', '--all-targets', '--', '-D', 'warnings'])]:
        execute('head-' + name, ROOT, command, env)


if __name__ == '__main__':
    main()
