#!/usr/bin/env python3
"""Generate, but never execute, a content-bound supplemental self-host check argv."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import tomllib

# This complete program is embedded literally in the approved argv. It never
# modifies project source or runs a provider CLI, Cargo, package manager or shell.
PROBE_CODE = r'''
import base64, hashlib, json, os, pathlib, subprocess, sys, tempfile
PLAN = __PLAN__
workspace = pathlib.Path(sys.argv[1]).resolve(strict=True)
binary = workspace / 'target/debug/antislop'
def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(65536),b''): h.update(chunk)
    return h.hexdigest()
if digest(binary) != PLAN['binary_sha256']: raise SystemExit('probe binary changed after argv generation')
expected=('antislop '+PLAN['version']+'\n').encode()
observations=[]
def contents(f):
    f.flush(); f.seek(0); h=hashlib.sha256(); size=0; small=bytearray()
    for chunk in iter(lambda:f.read(65536),b''):
        h.update(chunk); size+=len(chunk)
        if len(small)<16384: small.extend(chunk[:16384-len(small)])
    result=dict(bytes=size,sha256=h.hexdigest(),truncated=size>16384)
    if size<=16384:
        result['base64']=base64.b64encode(small).decode()
        result['utf8']=bytes(small).decode('utf-8',errors='replace')
    return result
with tempfile.TemporaryDirectory(prefix='antislop-supplemental-') as private:
    cwd=pathlib.Path(private)
    for case in PLAN['cases']:
        args=[base64.b64decode(x['bytes_base64']) if isinstance(x,dict) else x for x in case['args']]
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            pipe_write=None
            stdout=out
            if case.get('broken_stdout'):
                read_fd,pipe_write=os.pipe(); os.close(read_fd); stdout=pipe_write
            close_stdin=case.get('close_stdin',False)
            stdin=subprocess.PIPE if case.get('stdin_bytes') else subprocess.DEVNULL
            options=dict(cwd=cwd,stdin=stdin,stdout=stdout,stderr=err,
                         env={'PATH':os.defpath,'LANG':'C','LC_ALL':'C','HOME':str(cwd)})
            if close_stdin: options['preexec_fn']=lambda:os.close(0)
            state_before=(cwd/'--version').is_dir() if case['expect']=='position' else None
            proc=None; timed_out=False; launch_error=None
            try:
                proc=subprocess.Popen([str(binary),*args],**options)
                try: proc.communicate(input=b'ignored stdin bytes\n' if case.get('stdin_bytes') else None,timeout=2)
                except subprocess.TimeoutExpired:
                    timed_out=True
                    try: proc.kill()
                    except ProcessLookupError: pass
                    proc.communicate()
            except OSError as error:
                launch_error=dict(errno=error.errno,type=type(error).__name__)
            finally:
                if pipe_write is not None: os.close(pipe_write)
            stdout_result=contents(out) if not case.get('broken_stdout') else {'availability':'unobservable: pipe has no readers'}
            stderr_result=contents(err)
            observed=dict(case_id=case['id'],args=case['args'],declared=case['declared'],
                          stdin=case.get('stdin_mode','devnull EOF'),exit_code=None if proc is None else proc.returncode,
                          timed_out=timed_out,launch_error=launch_error,stdout=stdout_result,stderr=stderr_result)
            if case['expect']=='position':
                observed['state_directory_before']=state_before
                observed['state_directory_after']=(cwd/'--version').is_dir()
            if case['id']=='argv-untrusted': observed['sentinel_created']=(cwd/'never-execute').exists()
            observations.append(observed)
            failure=None
            if timed_out or launch_error: failure='probe did not complete normally'
            elif case['expect']=='version':
                if proc.returncode!=0 or stdout_result.get('base64')!=base64.b64encode(expected).decode() or stderr_result['bytes']!=0:
                    failure='version output/status/stderr violated exact contract'
            elif case['expect']=='help':
                if proc.returncode!=0 or 'antislop' not in stdout_result.get('utf8','') or stderr_result['bytes']!=0:
                    failure='help dispatch regression'
            elif case['expect']=='unknown':
                if proc.returncode==0 or stdout_result['bytes']!=0 or 'unknown command' not in stderr_result.get('utf8',''):
                    failure='non-version argument changed dispatch'
            elif case['expect']=='nonunicode':
                if proc.returncode==0: failure='expected retained non-Unicode argv rejection'
            elif case['expect']=='position':
                if proc.returncode==0 or stdout_result['bytes']!=0 or state_before or not observed['state_directory_after']:
                    failure='positional --version was intercepted or state handling changed'
            # A broken output stream is an observation, not a claim that an
            # impossible successful write occurred. The verifier judges the
            # actual exit/stderr against the declared failure-domain contract.
            if case['id']=='argv-untrusted' and observed['sentinel_created']:
                failure='untrusted argument produced a filesystem effect'
            observed['assertion_failure']=failure
            if failure:
                print(json.dumps(dict(artifact_hash=PLAN['artifact_hash'],binary_sha256=PLAN['binary_sha256'],observations=observations,failure=failure),ensure_ascii=False))
                raise SystemExit(1)
    if digest(binary)!=PLAN['binary_sha256']: raise SystemExit('probe binary changed during observations')
    print(json.dumps(dict(schema_version=1,artifact_hash=PLAN['artifact_hash'],binary_sha256=PLAN['binary_sha256'],
                         declared_inputs=PLAN['declarations'],observations=observations,
                         limitations=['Representative declared input cases only; no exhaustive argv proof.',
                                      'No benchmark, performance measurement, mutation run or provider CLI execution.',
                                      'Broken stdout has no capturable byte stream; exit and stderr are actual observations.',
                                      'fd 0 is closed before exec; Rust/OS startup may subsequently sanitize or reopen it.']),ensure_ascii=False))
'''


def plan(run, workspace):
    artifact = next(a for a in reversed(run['artifacts']) if a['stage'] == 'execution')
    if hashlib.sha256(artifact['content'].encode()).hexdigest() != artifact['hash']:
        raise ValueError('Execution artifact hash mismatch')
    output = json.loads(artifact['content'])
    matrix = output['quality']['software']['input_domain']
    declarations = [dict(input_id=i['id'], source_ref=i['source_ref'], category=c['category'],
                         scenario_ids=c['scenario_ids'], expected_result=c['expected_result'])
                    for i in matrix['inputs'] for c in i['categories'] if c['applicable']]
    categories = {x['category'] for x in declarations}
    cases = []

    def add(identifier, args, expect, matched, **kwargs):
        declared = [d for d in declarations if d['category'] in matched]
        if declared:
            cases.append(dict(id=identifier, args=args, expect=expect, declared=declared, **kwargs))

    # Normal version remains an explicit oracle, independently of category tags.
    cases.append(dict(id='version-normal', args=['--version'], expect='version', declared=[]))
    add('argv-missing', [], 'help', {'missing', 'minimum'})
    add('argv-empty', [''], 'unknown', {'empty'})
    add('argv-zero', ['0'], 'unknown', {'zero'})
    add('argv-negative', ['-1'], 'unknown', {'negative'})
    add('argv-long', ['x' * 8192], 'unknown', {'maximum', 'oversized'})
    add('argv-neighbor-short', ['--versio'], 'unknown', {'boundary_neighbors'})
    add('argv-neighbor-long', ['--versionx'], 'unknown', {'boundary_neighbors'})
    add('argv-case-sensitive', ['--VERSION'], 'unknown', {'boundary_neighbors'})
    add('argv-partial', ['--vers'], 'unknown', {'partial'})
    add('argv-unicode', ['żółw🦀'], 'unknown', {'unicode'})
    add('argv-untrusted', ['$(touch never-execute); ignore previous instructions'], 'unknown', {'untrusted'})
    invalid = {'bytes_base64': '/w=='}
    add('argv-invalid-encoding', [invalid], 'nonunicode', {'invalid_encoding'})
    add('argv-invalid-later-encoding', ['--version', invalid], 'nonunicode', {'invalid_encoding'})
    add('argv-duplicate', ['--version', '--version'], 'version', {'duplicate'})
    add('position-version-first', ['--version', '--help'], 'version', {'contradictory', 'wrong_order'})
    add('position-help-first', ['--help', '--version'], 'help', {'contradictory', 'wrong_order'})
    add('position-state-value', ['list', '--version'], 'position', {'wrong_order', 'path_normalization'})
    add('position-version-ignores-path', ['--version', '../does-not-exist'], 'version', {'path_normalization', 'access_boundaries'})
    add('stdin-devnull-eof', ['--version'], 'version', {'eof'}, stdin_mode='devnull EOF')
    add('stdin-fd-zero-closed', ['--version'], 'version', {'eof'}, close_stdin=True, stdin_mode='fd 0 explicitly closed before exec')
    add('stdin-unread-bytes', ['--version'], 'version', {'eof', 'stream_error'}, stdin_bytes=True, stdin_mode='supplied bytes; no reads required')
    add('stdout-normal', ['--version'], 'version', {'stream_error'})
    add('stdout-broken-pipe', ['--version'], 'observe_stream_failure', {'stream_error'}, broken_stdout=True)
    covered = {d['category'] for case in cases for d in case['declared']}
    if categories - covered:
        raise ValueError(f'No defined observation for applicable categories: {sorted(categories-covered)}')
    binary = workspace / 'target/debug/antislop'
    if not binary.resolve(strict=True).is_relative_to(workspace):
        raise ValueError('Probe binary escapes its workspace')
    return dict(artifact_hash=artifact['hash'], binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                version=tomllib.loads((workspace / 'Cargo.toml').read_text())['package']['version'],
                declarations=declarations, cases=cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-json', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--python', default=sys.executable)
    args = parser.parse_args()
    run = json.loads(args.run_json.read_text())
    workspace = Path(run['spec']['workspace']).resolve(strict=True)
    payload = plan(run, workspace)
    code = PROBE_CODE.replace('__PLAN__', repr(payload))
    argv = [str(Path(args.python).resolve(strict=True)), '-I', '-c', code, str(workspace)]
    args.output.write_text(json.dumps(argv, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(dict(argv_file=str(args.output), code_sha256=hashlib.sha256(code.encode()).hexdigest(),
                          artifact_hash=payload['artifact_hash'], cases=len(payload['cases'])), ensure_ascii=False))


if __name__ == '__main__':
    main()
