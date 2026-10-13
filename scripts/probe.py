#!/usr/bin/env python3
"""Read-only CLI discovery and a real app-server handshake; never dump auth data."""
import json
import os
import pathlib
import selectors
import shutil
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]

def call(argv):
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=20)
        return result.returncode, result.stdout
    except (OSError, subprocess.TimeoutExpired):
        return None, ''

def app_server(binary):
    process = subprocess.Popen([binary, 'app-server', '--stdio', '--disable', 'hooks', '--disable', 'plugins', '--disable', 'multi_agent'],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, start_new_session=True)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    def rpc(number, method, params):
        process.stdin.write(json.dumps(dict(id=number, method=method, params=params))+'\n')
        process.stdin.flush()
        deadline = time.monotonic()+20
        while time.monotonic()<deadline:
            if not selector.select(.25):
                continue
            line = process.stdout.readline()
            if not line:
                raise RuntimeError('app-server EOF')
            message = json.loads(line)
            if message.get('id') == number:
                if 'error' in message:
                    raise RuntimeError(f"RPC rejected: {method}")
                return message['result']
        raise RuntimeError('probe timed out')
    try:
        result = rpc(1, 'initialize', dict(clientInfo=dict(name='architect_m0', version='0.1.0')))
        process.stdin.write(json.dumps(dict(method='initialized', params={}))+'\n'); process.stdin.flush()
        # No model turn is started by this probe.
        thread = rpc(2, 'thread/start', dict(cwd=str(ROOT), sandbox='read-only', approvalPolicy='never',
                    config={'features.multi_agent':False, 'features.hooks':False, 'features.plugins':False}))
        return dict(initialize='observed', thread_start='observed', sandbox=thread.get('sandbox'),
                    session_id_present=bool(thread.get('thread',{}).get('id')), model_turn='not_run',
                    user_agent_present=bool(result.get('userAgent')))
    finally:
        selector.close()
        import signal
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()

def main():
    report = dict(host=os.uname().sysname, generated_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),providers={})
    for name in ['codex','claude','gemini']:
        binary = os.environ.get("GEMINI_BINARY") if name=="gemini" else None
        binary = binary or shutil.which(name)
        if binary is None:
            report['providers'][name] = dict(installed=False, capabilities='unverified', blocker='binary unavailable')
            continue
        status, version = call([binary,'--version'])
        _, help_text = call([binary,'--help'])
        auth = 'unverified'
        if name=='codex':
            code,_ = call([binary,'login','status']); auth='available' if code==0 else 'unavailable'
        if name=='claude':
            code,output=call([binary,'auth','status'])
            try: auth='available' if json.loads(output).get('loggedIn') else 'unavailable'
            except (ValueError,AttributeError): pass
        item=dict(installed=True, version=version.strip() if status==0 else 'unknown',auth=auth,
                  advertised_flags=[flag for flag in ['--resume','--json','--output-format','--input-format','--tools','--sandbox','--permission-mode'] if flag in help_text],
                  live_resume='unverified', live_cancel='unverified', linux='not_tested')
        if name=='codex':
            try: item['app_server']=app_server(binary)
            except Exception as error: item['app_server']=dict(status='failed',reason=str(error))
        report['providers'][name]=item
    path=ROOT/'docs/m0/probe.json'
    path.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))

if __name__=='__main__': main()
