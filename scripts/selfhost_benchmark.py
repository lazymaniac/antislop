#!/usr/bin/env python3
"""Generate content-bound benchmark argv for an explicitly supervised local check."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import sys
import tomllib

BENCHMARK_CODE = r'''
import base64, hashlib, json, math, os, pathlib, platform, random, signal, subprocess, sys, tempfile, time
PLAN = __PLAN__
def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(65536),b''): h.update(chunk)
    return h.hexdigest()
def sources(root):
    result={}
    for path in root.rglob('*'):
        relative=path.relative_to(root)
        if relative.parts[0]=='target' or '__pycache__' in relative.parts: continue
        if path.is_symlink(): raise RuntimeError('source symlink is outside benchmark scope')
        if path.is_file(): result[str(relative)]=sha(path)
    return result
def validate_files():
    files=PLAN['baseline_manifest'].get('source_hashes',PLAN['baseline_manifest'])
    for name in ('baseline','current'):
        if sha(pathlib.Path(PLAN[name]['binary']))!=PLAN[name]['sha256']: raise RuntimeError(name+' binary changed')
        expected=dict(files)
        if name=='current': expected['src/main.rs']=PLAN['current_main_sha256']
        if sources(pathlib.Path(PLAN[name]['source']))!=expected: raise RuntimeError(name+' sources changed')
validate_files()
if sys.platform not in ('darwin','linux'): raise SystemExit('unsupported wait4 RSS units')
def stream(f):
    f.flush(); f.seek(0); h=hashlib.sha256(); size=0
    for chunk in iter(lambda:f.read(65536),b''): h.update(chunk); size+=len(chunk)
    return dict(bytes=size,sha256=h.hexdigest())
def distribution(values):
    ordered=sorted(values)
    def percentile(p):
        index=(len(ordered)-1)*p; low=math.floor(index); high=math.ceil(index)
        return ordered[low]+(ordered[high]-ordered[low])*(index-low)
    return dict(n=len(values),p50=percentile(.5),p95=percentile(.95),p99=percentile(.99),max=max(values))
observations=[]
METRICS=('wall_ns','cpu_user_seconds','cpu_system_seconds','peak_rss_bytes','exit_code')
def summary(rows):
    return {name:distribution([row[name] for row in rows]) for name in ('wall_ns','cpu_user_seconds','cpu_system_seconds','peak_rss_bytes')}
with tempfile.TemporaryDirectory(prefix='antislop-benchmark-') as private:
    cwd=pathlib.Path(private)
    env=dict(PATH=os.defpath,HOME=private,LANG='C',LC_ALL='C',TZ='UTC')
    def measure(name,args):
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            start=time.perf_counter_ns()
            proc=subprocess.Popen([PLAN[name]['binary'],*args],cwd=cwd,env=env,stdin=subprocess.DEVNULL,stdout=out,stderr=err)
            deadline=start+int(PLAN['timeout_seconds']*1e9); timed_out=False
            while True:
                waited,status,usage=os.wait4(proc.pid,os.WNOHANG)
                if waited: break
                if time.perf_counter_ns()>=deadline:
                    timed_out=True
                    # wait4 just reported an unreaped child: this PID cannot be
                    # reused before our subsequent wait4. Never signal a group.
                    try: os.kill(proc.pid,signal.SIGKILL)
                    except ProcessLookupError: pass
                    _,status,usage=os.wait4(proc.pid,0)
                    break
                time.sleep(PLAN['poll_seconds'])
            elapsed=time.perf_counter_ns()-start
            proc.returncode=os.waitstatus_to_exitcode(status)
            row=dict(binary=name,wall_ns=elapsed,cpu_user_seconds=usage.ru_utime,
                     cpu_system_seconds=usage.ru_stime,
                     peak_rss_bytes=int(usage.ru_maxrss)*(1 if sys.platform=='darwin' else 1024),
                     exit_code=proc.returncode,timed_out=timed_out,stdout=stream(out),stderr=stream(err))
            if timed_out:
                observations.append(dict(workload='timed-out-child',sample=row))
                # Abort immediately: the outer Check guardian then cleans all
                # same-group descendants. No setsid/new process group is used.
                raise RuntimeError('individual child timeout; supervised runner must finish group cleanup')
            return row
    rng=random.Random(PLAN['seed']); workloads=[]
    try:
        for workload in PLAN['workloads']:
            pairs=[]; orders=['baseline','current']*(PLAN['pairs']//2); rng.shuffle(orders)
            canonical=None
            for index,first in enumerate(orders):
                order=[first,'current' if first=='baseline' else 'baseline']; pair={}
                for name in order: pair[name]=measure(name,workload['args'])
                row=dict(pair=index,order=order,baseline=pair['baseline'],current=pair['current'])
                pairs.append(row)
                observations.append(dict(workload=workload['id'],pair=row))
                for name in ('baseline','current'):
                    result=pair[name]
                    if result['exit_code']!=workload['exit_code']: raise RuntimeError('unexpected existing-command exit code')
                    if workload['id']=='help' and result['stderr']['bytes']!=0: raise RuntimeError('help stderr regression')
                    if workload['id']!='help' and result['stdout']['bytes']!=0: raise RuntimeError('unknown-command stdout regression')
                if any(pair['baseline'][stream]!=pair['current'][stream] for stream in ('stdout','stderr')):
                    raise RuntimeError('existing-command stdout/stderr differ from baseline')
                output={key:pair['baseline'][key] for key in ('stdout','stderr')}
                if canonical is None: canonical=output
                elif output!=canonical: raise RuntimeError('existing-command output changed between samples')
            columns=['first_binary']+[name+'_'+metric for name in ('baseline','current') for metric in METRICS]
            samples=[[pair['order'][0]]+[pair[name][metric] for name in ('baseline','current') for metric in METRICS] for pair in pairs]
            result=dict(id=workload['id'],argv_bytes=workload.get('argv_bytes'),sample_columns=columns,samples=samples,
                        verified_output=canonical,
                        baseline=summary([pair['baseline'] for pair in pairs]),current=summary([pair['current'] for pair in pairs]),
                        paired_wall_ratio=distribution([pair['current']['wall_ns']/pair['baseline']['wall_ns'] for pair in pairs]))
            workloads.append(result)
        versions=[]
        expected=('antislop '+PLAN['version']+'\n').encode()
        for workload in PLAN['version_workloads']:
            rows=[]
            for index in range(PLAN['pairs']):
                row=measure('current',workload['args']);rows.append(row);observations.append(dict(workload=workload['id'],sample=row))
                if row['exit_code']!=0 or row['stdout']!={'bytes':len(expected),'sha256':hashlib.sha256(expected).hexdigest()} or row['stderr']['bytes']!=0:
                    raise RuntimeError('version exact output/status/stderr contract failed')
            versions.append(dict(id=workload['id'],trailing_argv_bytes=workload['trailing_argv_bytes'],sample_columns=list(METRICS),
                samples=[[row[key] for key in METRICS] for row in rows],
                verified_output={key:rows[0][key] for key in ('stdout','stderr')},summary=summary(rows),comparable_baseline=False))
        validate_files()
        print(json.dumps(dict(schema_version=1,plan=PLAN,host=dict(platform=sys.platform,machine=platform.machine(),python=platform.python_version(),cpu_count=os.cpu_count(),load_average=os.getloadavg()),
            measurements=workloads,version_current_only=versions,
            unknown_argv_scaling=[dict(argv_bytes=row['argv_bytes'],baseline_wall_ns=row['baseline']['wall_ns'],current_wall_ns=row['current']['wall_ns'],paired_wall_ratio=row['paired_wall_ratio']) for row in workloads if row['argv_bytes'] is not None],
            limitations=['Descriptive 30-pair samples only; p95/p99 interpolation is not a statistical confidence claim.',
                'Wall time includes process startup, scheduling and wait4 polling (250 microseconds); no isolated branch latency measurement.',
                'No warmup or cache flushing; randomized balanced pair order reduces but does not remove host/cache effects.',
                'Per-child wait4 peak RSS includes launcher/pre-exec accounting; it is not steady-state memory or an allocation profile.',
                'macOS ru_maxrss bytes and Linux KiB are normalized to bytes; no global RUSAGE_CHILDREN aggregation.',
                'Version has no equivalent successful baseline command; candidate-only samples cannot establish a speedup.',
                'Binary/source hashes bind supplied build artifacts; compiler identity/build flags require retained build provenance.',
                'Individual timeout aborts the batch; enclosing supervised Check cleans same-group descendants. No detached-process containment claim.',
                'No performance pass/fail threshold or exhaustive OS argument-limit claim.']),separators=(',',':')))
    except (RuntimeError,OSError) as error:
        print(json.dumps(dict(schema_version=1,plan=PLAN,observations=observations,failure=str(error)),separators=(',',':')))
        raise SystemExit(1)
'''


def fingerprint(root):
    result = {}
    for path in root.rglob('*'):
        relative = path.relative_to(root)
        if relative.parts[0] == 'target' or '__pycache__' in relative.parts:
            continue
        if path.is_symlink():
            raise ValueError('source symlink is outside benchmark scope')
        if path.is_file():
            result[str(relative)] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def plan(baseline, current, manifest_path, timeout=2.0):
    binaries = {}
    for name, path in (('baseline', baseline), ('current', current)):
        binary = path.resolve(strict=True)
        if binary.parent.name != 'debug' or binary.parent.parent.name != 'target':
            raise ValueError('supplied binary must be SOURCE/target/debug/antislop')
        source = binary.parents[2]
        binaries[name] = dict(binary=str(binary), sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                              source=str(source), source_hashes=fingerprint(source))
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw)
    if not isinstance(manifest, dict):
        raise ValueError('baseline manifest must be an object')
    files = manifest.get('source_hashes', manifest)
    if not isinstance(files, dict) or not files or binaries['baseline']['source_hashes'] != files:
        raise ValueError('baseline source manifest does not match supplied baseline source')
    before, after = binaries['baseline']['source_hashes'], binaries['current']['source_hashes']
    changed = sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))
    if changed != ['src/main.rs']:
        raise ValueError('benchmark source difference must be only src/main.rs')
    if 'binary_sha256' in manifest and manifest['binary_sha256'] != binaries['baseline']['sha256']:
        raise ValueError('baseline manifest binary SHA mismatch')
    if not 0 < timeout <= 60:
        raise ValueError('individual timeout must be positive and at most 60 seconds')
    current_main = after['src/main.rs']
    for binary in binaries.values():
        binary.pop('source_hashes')
    return dict(**binaries, current_main_sha256=current_main,
                baseline_manifest=manifest, baseline_manifest_base64=base64.b64encode(raw).decode(),
                baseline_manifest_sha256=hashlib.sha256(raw).hexdigest(), pairs=30, seed=20261013,
                timeout_seconds=timeout, poll_seconds=.00025,
                version=tomllib.loads((Path(binaries['current']['source']) / 'Cargo.toml').read_text())['package']['version'],
                workloads=[dict(id='help', args=['--help'], exit_code=0)] +
                [dict(id=f'unknown-{size}', args=['x' * size], argv_bytes=size, exit_code=1) for size in (32, 256, 2048, 8192)],
                version_workloads=[dict(id='version-normal',args=['--version'],trailing_argv_bytes=0)] +
                [dict(id=f'version-trailing-{size}',args=['--version','x'*size],trailing_argv_bytes=size) for size in (32,256,2048,8192)])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--current', type=Path, required=True)
    parser.add_argument('--baseline-manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--python', default=sys.executable)
    parser.add_argument('--timeout', type=float, default=2.0)
    args = parser.parse_args()
    payload = plan(args.baseline, args.current, args.baseline_manifest, args.timeout)
    code = BENCHMARK_CODE.replace('__PLAN__', repr(payload))
    argv = [str(Path(args.python).resolve(strict=True)), '-I', '-c', code]
    args.output.write_text(json.dumps(argv, indent=2) + '\n')
    print(json.dumps(dict(argv_file=str(args.output), code_sha256=hashlib.sha256(code.encode()).hexdigest(),
                          baseline_binary_sha256=payload['baseline']['sha256'], current_binary_sha256=payload['current']['sha256'],
                          baseline_manifest_sha256=payload['baseline_manifest_sha256'], paired_workloads=5, pairs_per_workload=30,
                          current_version_workloads=5,current_version_samples_per_workload=30)))


if __name__ == '__main__':
    main()
