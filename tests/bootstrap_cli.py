#!/usr/bin/env python3
"""CLI refusal/recovery on real executable copies in a new durable fixture root."""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=pathlib.Path, required=True)
parser.add_argument('--legacy', type=pathlib.Path, required=True)
parser.add_argument('--root', type=pathlib.Path, required=True)
args = parser.parse_args()
binary = args.binary.resolve()
root = args.root.resolve()
root.mkdir(parents=True, exist_ok=False)
shutil.copy2(__file__, root / "bootstrap_cli.py")


def log(label, value):
    print(label, json.dumps(value), flush=True)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(argv, response=''):
    process = subprocess.run(list(map(str, argv)), input=response, capture_output=True, text=True, cwd=root)
    log('command', dict(argv=list(map(str, argv)), cwd=str(root), exit=process.returncode,
                        stdout=process.stdout, stderr=process.stderr))
    return process


def fixture(name):
    directory = root / name
    directory.mkdir()
    target = directory / 'frontend'
    shutil.copy2(args.legacy, target)
    journal = directory / 'journal'
    assert command([binary, 'bootstrap', 'prepare', '--target', target, '--candidate', binary,
                    '--journal', journal]).returncode == 0
    return target, journal


def confirm(action, journal, interrupted=False):
    argv = [str(binary), 'bootstrap', action, '--journal', str(journal), '--exclusions',
            'ISOLATED TEST: sole fixture operator excludes package managers, IDE, installers, all terminals; prevents writes through journal settlement']
    process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, cwd=root)
    prompt = ''
    for line in process.stderr:
        prompt += line
        if 'type: CONFIRM ' in line:
            challenge = line.split('type: CONFIRM ')[1].strip()
            if interrupted:
                process.terminate()  # Explicit fixture interruption before any destructive step.
            else:
                process.stdin.write('CONFIRM ' + challenge + '\n')
                process.stdin.flush()
            break
    stdout, stderr = process.communicate()
    log('window-operation', dict(argv=argv, cwd=str(root), exit=process.returncode,
                                 prompt=prompt, stdout=stdout, stderr=stderr, interrupted=interrupted))
    return process.returncode, challenge, stderr


target, journal = fixture('normal')
original = digest(target)
base = [binary, 'bootstrap', 'publish', '--journal', journal]
assert command(base).returncode == 1
for response in ['', 'CONFIRM stale-window\n']:
    result = command(base + ['--exclusions', 'isolated sole fixture writer'], response)
    assert result.returncode == 1 and 'missing or stale' in result.stderr
    assert digest(target) == original
exit_code, old_challenge, _ = confirm('publish', journal, interrupted=True)
assert exit_code < 0 and digest(target) == original
result = command([binary, 'bootstrap', 'resume', '--journal', journal, '--exclusions', 'renewed fixture exclusions'],
                 'CONFIRM ' + old_challenge + '\n')
assert result.returncode == 1 and 'missing or stale' in result.stderr
assert confirm('resume', journal)[0] == 0
assert digest(target) == digest(binary)
assert confirm('cancel', journal)[0] == 0
assert digest(target) == digest(binary)
assert confirm('resume', journal)[0] == 1
assert confirm('restore', journal)[0] == 0
assert digest(target) == original
for phase in ['before', 'after']:
    target, journal = fixture('user-change-' + phase)
    if phase == 'after':
        assert confirm('publish', journal)[0] == 0
    user = target.with_name('user-copy')
    shutil.copy2(args.legacy, user)
    with user.open('ab') as stream:
        stream.write(b'user-version')
    user.replace(target)
    changed = digest(target)
    for action in ['publish', 'resume', 'cancel', 'restore']:
        result, _, error = confirm(action, journal)
        assert result == 1 and 'target identity changed' in error
        assert digest(target) == changed
        assert (journal / 'original').exists() and (journal / 'candidate').exists()
    log('user-version-retained', dict(phase=phase, target=str(target), sha256=changed))
log('PASS', 'real CLI: missing/stale/interrupted windows; renewed resume; cancel; restore; pre/post-publication user changes retained')
