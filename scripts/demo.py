#!/usr/bin/env python3
"""Exercise the supervised protocol on a disposable, pre-authorized fixture.

The driver supplies explicit stage approvals only for this fixed demonstration.
It is not an autonomous-mode implementation or an approval bypass in the daemon.
"""
import argparse
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]

def rpc(state, operation, key=None):
    request = dict(version=2,request_id=str(uuid.uuid4()),idempotency_key=key,operation=operation)
    while True:
        try:
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as stream:
                stream.connect(str(state/'architect.sock'))
                stream.sendall((json.dumps(request)+'\n').encode())
                with stream.makefile('rb') as output:
                    response=json.loads(output.readline())
            break
        except (OSError,json.JSONDecodeError):
            # Reconnect repeats the exact request and idempotency key. A committed
            # mutation returns its receipt and never launches a second attempt.
            time.sleep(.1)
    if response['error']:
        raise RuntimeError(response['error'])
    data = response['data']
    if operation['type'] == 'snapshot':
        data['run'] = read_run(state, operation['run_id'])
    return data

def read_run(state,run_id):
    import hashlib
    while True:
        offset=0; digest=None; chunks=[]
        try:
            while True:
                data=rpc(state,dict(type='read',run_id=run_id,document='run',offset=offset,expected_hash=digest))
                digest=data['hash'];chunks.append(data['chunk'])
                if data['next_offset'] is None:break
                offset=data['next_offset']
        except RuntimeError as error:
            if 'document changed' not in str(error):raise
            continue # Read-only retry: never replay a mutation to refresh its view.
        text=''.join(chunks)
        assert hashlib.sha256(text.encode()).hexdigest()==digest
        return json.loads(text)

def mutate(state,command,key=None):
    return rpc(state,dict(type='mutate',command=command),key or str(uuid.uuid4()))

def fixture(root):
    repository=root/'fixture.git'
    subprocess.run(['git','init','--bare',str(repository)],check=True,capture_output=True)
    compiler=str(pathlib.Path.home()/'.cargo/bin/rustup')
    source='pub fn magnitude(value: i32) -> i64 { i64::from(value) }\n'
    tests='''#[path="lib.rs"] mod subject;
#[test] fn positive() { assert_eq!(subject::magnitude(7),7); }
#[test] fn zero() { assert_eq!(subject::magnitude(0),0); }
#[test] fn negative() { assert_eq!(subject::magnitude(-7),7); }
#[test] fn minimum() { assert_eq!(subject::magnitude(i32::MIN),2147483648); }
'''
    check='import subprocess\nsubprocess.run('+repr([compiler,'run','1.98.1','rustc','--test','tests.rs','-o','checks'])+',check=True)\nsubprocess.run(["./checks"],check=True)\n'
    files={'lib.rs':source,'tests.rs':tests,'check.py':check}
    data=b'commit refs/heads/main\ncommitter Architect Fixture <fixture@localhost> 1700000000 +0000\ndata 7\nfixture\n'
    for path,content in files.items():
        raw=content.encode(); data+=f'M 100644 inline {path}\ndata {len(raw)}\n'.encode()+raw+b'\n'
    data+=b'\ndone\n'
    subprocess.run(['git','-C',str(repository),'fast-import','--quiet'],input=data,check=True,capture_output=True)
    subprocess.run(['git','-C',str(repository),'symbolic-ref','HEAD','refs/heads/main'],check=True)
    workspace=root/'worktree'
    subprocess.run(['git','-C',str(repository),'worktree','add','--detach',str(workspace),'HEAD'],check=True,capture_output=True)
    baseline=subprocess.run([sys.executable,'check.py'],cwd=workspace,capture_output=True)
    assert baseline.returncode!=0,'fixture must reproduce bug'
    return workspace

def launch(binary,state):
    process=subprocess.Popen([str(binary),'daemon',str(state)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    for _ in range(100):
        if process.poll() is not None: raise RuntimeError('daemon failed to start')
        if (state/'architect.sock').exists():
            try: rpc(state,dict(type='list'));return process
            except (OSError,RuntimeError): pass
        time.sleep(.05)
    process.terminate();process.wait();raise RuntimeError('daemon startup timed out')

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--live',action='store_true',help='Use existing authenticated Codex CLI; consumes provider usage')
    parser.add_argument('--repair',action='store_true',help='Approve repair rounds only within this fixed disposable fixture')
    parser.add_argument('--reject',action='store_true',help='Seed a rejected fake artifact')
    args=parser.parse_args()
    binary=ROOT/'target/debug/architect'
    if not binary.exists(): raise SystemExit('Build first: scripts/cargo.sh build')
    with tempfile.TemporaryDirectory(prefix='architect-demo-') as temp:
        root=pathlib.Path(temp)
        workspace=fixture(root) if args.live else root
        state=root/'state';state.mkdir(mode=0o700)
        process=launch(binary,state)
        try:
            spec=dict(objective=('[reject] ' if args.reject else '')+'Fix magnitude for negative i32 inputs, including i32::MIN. Preserve positive and zero behavior. Change only lib.rs; do not change tests.',
                      workspace=str(workspace),provider='codex' if args.live else 'fake',model=None,
                      requirements=[dict(id='REQ-1',original_text='Return the nonnegative magnitude as i64 for every i32 input, including MIN; preserve zero and positive inputs.',source_ref='demo:objective',
                                         criteria=dict(negative='-7 returns 7',minimum='i32::MIN returns 2147483648',zero='0 returns 0',positive='7 returns 7'))],
                      plan_items=[dict(id='P1',requirement_ids=['REQ-1'],description='Widen to i64 before absolute value, preserve API, verify boundaries and tests.')],
                      input_files=['lib.rs','tests.rs','check.py'] if args.live else [],editable_files=['lib.rs'] if args.live else [],
                      check_command=[sys.executable,'check.py'] if args.live else [])
            run=mutate(state,dict(type='create',spec=spec));run_id=run['id']
            # A real daemon restart before approvals demonstrates durable reattach.
            process.terminate();process.wait();process=launch(binary,state)
            recovered=rpc(state,dict(type='snapshot',run_id=run_id));assert recovered['run']['status']=='awaiting_start'
            stages=[]
            while True:
                snapshot=rpc(state,dict(type='snapshot',run_id=run_id));run=snapshot['run']
                print(run['stage'],run['status'],flush=True)
                if run['status']=='completed':break
                assert run['status'] in ('awaiting_start','changes_requested','blocked'),run.get('diagnostic')
                repair=run['status'] in ('changes_requested','blocked')
                mutate(state,dict(type='approve_repair' if repair else 'approve_start',run_id=run_id,scope_hash=snapshot['scope_hashes']['repair' if repair else 'start']))
                key=str(uuid.uuid4());command=dict(type='advance',run_id=run_id)
                mutate(state,command,key)
                mutate(state,command,key) # exact duplicate must not create another attempt
                while True:
                    time.sleep(.25)
                    snapshot=rpc(state,dict(type='snapshot',run_id=run_id));run=snapshot['run']
                    if run['status']!='running':break
                if args.reject:
                    assert run['status']=='changes_requested',run.get('diagnostic')
                    try:mutate(state,dict(type='advance',run_id=run_id))
                    except RuntimeError:pass
                    else:raise AssertionError('repair started without approval')
                    print('seeded rejection observed; repair waits for approval',flush=True);break
                if run['status']!='awaiting_result':
                    print('Gate blocked:',run.get('diagnostic'),flush=True)
                    if run['artifacts']:
                        print('Artifact context:',json.loads(run['artifacts'][-1]['content']).get('context'),flush=True)
                    for review in run['reviews'][-1:]:
                        print('Findings:',review['findings'],flush=True)
                        print('Nonpassing checks:',[c['check_id'] for c in review['checks'] if c['status'] in ('fail','blocked')],flush=True)
                    if args.repair and run['status'] in ('changes_requested','blocked'):
                        print('Fixture explicitly approves the next repair round.',flush=True)
                        continue
                    raise RuntimeError('demo did not pass verification')
                stages.append(run['stage'])
                mutate(state,dict(type='accept_result',run_id=run_id,scope_hash=snapshot['scope_hashes']['accept']))
            report=dict(provider=spec['provider'],seeded_rejection=args.reject,status=run['status'],stages=stages,
                        artifacts=len(run['artifacts']),reviews=len(run['reviews']),approvals=len(run['approvals']),
                        independent_sessions=all(r['executor_session']!=r['verifier_session'] for r in run['reviews']),
                        daemon_restart_observed=True,duplicate_advance_observed=True)
            if args.live:
                result=subprocess.run([sys.executable,'check.py'],cwd=workspace,capture_output=True)
                report['final_tests_passed']=result.returncode==0
                report['fixed_source']=(workspace/'lib.rs').read_text()
                evidence_path=ROOT/'docs/m0/live-evidence.json'
                evidence_path.write_text(json.dumps(dict(schema_version=1,run=run),indent=2)+'\n')
                report['evidence_file']='docs/m0/live-evidence.json'
                assert result.returncode==0
            (ROOT/'docs/m0'/('live-demo.json' if args.live else 'rejection-demo.json' if args.reject else 'fake-demo.json')).write_text(json.dumps(report,indent=2)+'\n')
            print(json.dumps(report,indent=2))
        finally:
            process.terminate();process.wait()

if __name__=='__main__':main()
