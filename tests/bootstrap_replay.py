#!/usr/bin/env python3
"""Real legacy Events frontend + conditional publication + daemon-owned handoff.

Active fixtures use short external temporary directories; NEW --root retains diagnostics. --legacy is an
archived actual old executable, not a newly built imitation of the old client.
The operator owns this isolated fixture and excludes all other writers.
"""
import argparse
from contextlib import ExitStack
import pathlib
import sys
sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "support"))
from resources import external, resources, process as owned_process
from replay_cleanup import restore_listener, stop_daemon, cleanup_replay
import hashlib
import json
import os
import pathlib
import shutil
import socket
import sqlite3
import subprocess
import threading
import time
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--legacy', type=pathlib.Path, required=True)
parser.add_argument('--candidate', type=pathlib.Path, required=True)
parser.add_argument('--loss-phase', choices=['switching', 'start'], default='start')
parser.add_argument('--root', type=pathlib.Path, required=True)
args = parser.parse_args()

def main():
    evidence = external(args.root, strict=False)
    evidence.mkdir(parents=True, exist_ok=False)
    with resources(diagnostic_root=evidence) as (active, diagnostic, env), ExitStack() as stack, ExitStack() as daemon_cleanup:
        source = pathlib.Path(__file__).resolve().parent.parent
        r = active
        shutil.copy2(__file__, r / "bootstrap_replay.py")
        a, b = r / 'frontend-A', args.candidate.resolve()
        shutil.copy2(args.legacy, a)
        project = r / 'project'
        project.mkdir()
        subprocess.run(['git', 'init', '-q', '--initial-branch=main'], cwd=project, check=True)
        subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                        'commit', '-q', '--allow-empty', '-m', 'fixture'], cwd=project, check=True)
        config = r / 'antislop.toml'
        config.write_text((source / 'examples/antislop.toml').read_text().replace(
            'binary = "codex"', 'binary = "/usr/bin/true"').replace(
            'binary = "agy"', 'binary = "/usr/bin/true"'))
        state = r / 'state'
        env = dict(env, ANTISLOP_STATE_ROOT=str(r / 'global-state'),
                   ANTISLOP_CONFIG_DIR=str(r / 'global-config'))
        protocol = json.loads(subprocess.check_output([str(b), '--version', '--json']))['protocol']
        assert json.loads(subprocess.check_output([str(a), '--version', '--json']))['protocol'] == protocol
        sock = str(state / 'core.sock')
        trace_lock = threading.Lock()


        def log(label, value):
            with trace_lock:
                line = label + ' ' + json.dumps(value)
                with (diagnostic / 'fixture.log').open('a') as output:
                    output.write(line + '\n')
                print(line, flush=True)


        def command(argv, **kwargs):
            result = subprocess.run(list(map(str, argv)), cwd=project, env=env,
                                    text=True, capture_output=True, **kwargs)
            log('command', dict(argv=list(map(str, argv)), cwd=str(project), exit=result.returncode,
                                stdout=result.stdout, stderr=result.stderr))
            return result


        def rpc(op, **fields):
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(3)
                stream.connect(sock)
                stream.sendall((json.dumps(dict(protocol=protocol, client=None,
                                               request=dict(op=op, **fields))) + '\n').encode())
                response = json.loads(stream.makefile('rb').readline())
                if response.get('error'):
                    raise RuntimeError(response['error'])
                return response['data']


        def wait(predicate):
            deadline = time.monotonic() + 60
            last = None
            while time.monotonic() < deadline:
                try:
                    value = predicate()
                    if value:
                        return value
                except (OSError, RuntimeError, json.JSONDecodeError) as error:
                    last = str(error)
                time.sleep(.005)
            raise AssertionError(f'fixture observation deadline: {last}')


        def publication(action, journal):
            argv = [str(b), 'bootstrap', action, '--journal', str(journal), '--exclusions',
                    'ISOLATED TEST WINDOW: fixture exclusively owned by this harness; no package manager, IDE, installer, or other terminal targets fixture; harness prevents all other writes through durable settlement']
            process = stack.enter_context(owned_process(argv, cwd=project, env=env, text=True, stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE))
            prompt = ''
            for line in process.stderr:
                prompt += line
                if 'type: CONFIRM ' in line:
                    challenge = line.split('type: CONFIRM ')[1].strip()
                    log('window-established', dict(action=action, target=str(a), journal=str(journal),
                                                   challenge=challenge, preconditions=argv[-1]))
                    process.stdin.write('CONFIRM ' + challenge + '\n')
                    process.stdin.flush()
                    break
            stdout, stderr = process.communicate()
            log('publication', dict(argv=argv, cwd=str(project), exit=process.returncode,
                                    prompt=prompt, stdout=stdout, stderr=stderr))
            assert process.returncode == 0


        # Preserve and correlate the executable identities before touching fixture A.
        for path in [a, b]:
            log('binary', dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                               version=json.loads(subprocess.check_output([str(path), '--version', '--json']))))
        daemon_cleanup.callback(lambda: stop_daemon(command, b, state))
        front = stack.enter_context(owned_process([str(a), 'chat', '--config', str(config), '--state', str(state)],
                                 cwd=project, env=env, stdin=subprocess.PIPE,
                                 stdout=stack.enter_context(open(r / 'frontend.stdout', 'w')), stderr=stack.enter_context(open(r / 'frontend.stderr', 'w'))))
        continuity_failure = False
        held = threading.Event()
        release = threading.Event()
        proxy_stop = threading.Event()
        proxy = None
        upstream = pathlib.Path(sock + '.upstream')
        upstream_identity = None
        proxy_identity = None
        accept_worker = None
        workers = []
        try:
            original = wait(lambda: rpc('identity'))
            log('original', original)
            wait(lambda: 'conversation ' in (r / 'frontend.stderr').read_text())
            log('frontend', dict(pid=front.pid, executable=str(a)))
            conversation = rpc('start')['id']
            rpc('pause', conversation=conversation, paused=True)
            operation = dict(conversation=conversation, operation_id=str(uuid.uuid4()),
                             text='retained paused message before bootstrap')
            accepted = rpc('send', **operation)
            log('durable-operation-before', accepted)
            history = rpc('history', conversation=conversation)
            assert 'retained paused message' in json.dumps(history)

            # Real exec failure after successful validation: mode is not included in
            # runtime build identity, so this reaches exec rather than check_inputs.
            denied = r / 'exec-denied'
            shutil.copy2(b, denied)
            assert command([b, 'activate', '--binary', denied, '--config', config, '--state', state, '--request-id', 'fixture-exec-failure']).returncode == 0
            wait(lambda: rpc('runtime_activation_status')['state'] == 'waiting_for_idle')
            denied.chmod(0o600)
            failed = wait(lambda: (value if (value := rpc('runtime_activation_status'))['state'] in ['applied', 'failed', 'interrupted'] else None))
            log('real-exec-failure', {key: failed[key] for key in ['id', 'state', 'error', 'executable', 'build']})
            assert failed['state'] == 'failed' and 'Permission denied' in failed['error']
            assert rpc('identity')['build'] == original['build']

            # Interpose a fixture-only socket relay. Hold a real old frontend's Events
            # response and close it after durable Switching and successor listener rebind.
            upstream_identity = os.stat(sock)
            os.rename(sock, upstream)
            proxy = socket.socket(socket.AF_UNIX)
            proxy.bind(sock)
            proxy_identity = os.stat(sock)
            proxy.listen()
            proxy_inode = os.stat(sock).st_ino
            proxy.settimeout(.1)

            def relay(frontend):
                try:
                    with frontend, socket.socket(socket.AF_UNIX) as backend:
                        request = frontend.makefile('rb').readline()
                        envelope = json.loads(request)
                        backend.connect(str(upstream))
                        backend.sendall(request)
                        response = backend.makefile('rb').readline()
                        op = envelope['request']['op']
                        if op in ('events', 'events_paged') and not held.is_set():
                            log('held-real-events', envelope)
                            held.set()
                            release.wait()
                            frontend.shutdown(socket.SHUT_RDWR)
                            log('events-lost-after-switching', dict(frontend_pid=front.pid))
                        else:
                            log('relay-request', dict(op=op))
                            frontend.sendall(response)
                except (OSError, json.JSONDecodeError) as error:
                    log('relay-ended', str(error))

            def accept():
                while not proxy_stop.is_set():
                    try:
                        connection, _ = proxy.accept()
                        worker = threading.Thread(target=relay, args=(connection,))
                        workers.append(worker)
                        worker.start()
                    except socket.timeout:
                        continue
                    except OSError:
                        break

            accept_worker = threading.Thread(target=accept)
            accept_worker.start()
            assert held.wait(10), 'actual legacy Events did not reach relay'
            journal = r / 'publication'
            assert command([b, 'bootstrap', 'prepare', '--target', a, '--candidate', b,
                            '--journal', journal]).returncode == 0
            publication('publish', journal)
            final = json.loads(subprocess.check_output([str(a), '--version', '--json']))
            log('final-file', dict(path=str(a), sha256=hashlib.sha256(a.read_bytes()).hexdigest(), version=final))
            request_id = 'fixture-published-handoff'

            def switching():
                with sqlite3.connect(f'file:{state}/core.db?mode=ro', uri=True) as db:
                    return db.execute("SELECT state FROM runtime_activation_events WHERE operation=? ORDER BY rowid", (request_id,)).fetchall()

            def observe_switch():
                rows = wait(lambda: (rows if ('switching',) in (rows := switching()) else None))
                log('durable-switching', rows)
                if args.loss_phase == "start":
                    wait(lambda: os.stat(sock).st_ino != proxy_inode)
                log('loss-boundary', dict(requested=args.loss_phase, socket_inode=os.stat(sock).st_ino, states=switching()))
                release.set()

            observer = threading.Thread(target=observe_switch)
            workers.append(observer)
            observer.start()
            assert command([b, 'activate', '--binary', a, '--config', config, '--state', state,
                            '--request-id', request_id]).returncode == 0
            observer.join(60)
            assert not observer.is_alive() and release.is_set()
            settled = wait(lambda: (value if (value := rpc('runtime_activation_status'))['state'] in
                                    ['applied', 'failed', 'interrupted'] else None))
            log('settled', settled)
            assert settled['state'] == 'applied' and settled['build'] == final['build']
            live = wait(lambda: rpc('identity'))
            log('live', live)
            assert live['build'] == final['build']
            assert live['pid'] == original['pid'], 'handoff must exec the same daemon'
            if args.loss_phase == "switching":
                time.sleep(.1)
            if front.poll() is not None:
                continuity_failure = True
                log('legacy-frontend-continuity-failure', dict(exit=front.returncode, raw=(r / 'frontend.stderr').read_text()))
                assert args.loss_phase == 'switching', 'unexpected frontend failure after successor start'
                previous_frontend = front
                continued_conversation = (r / 'frontend.stderr').read_text().split('conversation ')[1].splitlines()[0]
                front = stack.enter_context(owned_process([str(a), 'chat', '--config', str(config), '--state', str(state), '--conversation', continued_conversation], cwd=project, env=env, stdin=subprocess.PIPE, stdout=stack.enter_context(open(r / 'continuation.stdout', 'w')), stderr=stack.enter_context(open(r / 'continuation.stderr', 'w'))))
                wait(lambda: 'conversation ' in (r / 'continuation.stderr').read_text())
                assert rpc('identity')['pid'] == original['pid']
                log('supported-continuation', dict(argv=[str(a), 'chat', '--config', str(config), '--state', str(state), '--conversation', continued_conversation], frontend_pid=front.pid, daemon_pid=original['pid'], old_frontend_exit=previous_frontend.returncode))
            assert front.poll() is None
            contender = command([b, '__core', project, config, state])
            assert contender.returncode == 1 and 'another antislop process owns' in contender.stderr
            # Durable pause and idempotent operation survive Core::open in B.
            assert rpc('send', **operation) == accepted
            after = rpc('history', conversation=conversation)
            assert after['events'][:len(history['events'])] == history['events']
            assert next(item for item in rpc('conversations') if item['id'] == conversation)['status'] == 'paused'
            log('durable-history-retained', dict(original_event_count=len(history['events']), current_event_count=len(after['events']), paused=True))
            log('durable-operation-after', rpc('send', **operation))
            log('conversations-after', rpc('conversations'))
            assert not rpc('identity')['busy']
            assert command([b, 'activate', '--status', '--request-id', request_id, '--state', state]).returncode == 0
            log('activation-events', switching())
            log('live-process', subprocess.check_output(['ps', '-p', str(live['pid']), '-o', 'pid,ppid,command'], text=True))
            log('live-text', subprocess.check_output(['lsof', '-a', '-p', str(live['pid']), '-d', 'txt', '-Fn'], text=True))
            # Reproduce the historical later-loss baseline. Only the frontend starts
            # the replacement; this harness never manually starts a second daemon.
            log('fault', dict(signal='SIGTERM', pid=live['pid'], verified_project=live['project'],
                              verified_config=live['config'], purpose='isolated later Events loss'))
            assert live['project'] == str(project) and live['config'] == str(config)
            os.kill(live['pid'], 15)
            reopened = wait(lambda: (value if (value := rpc('identity'))['pid'] != live['pid'] else None))
            log('after-later-loss', reopened)
            assert reopened['build'] == final['build'] != original['build']
            assert rpc('runtime_activation_status')['state'] == 'applied'
            assert rpc('send', **operation) == accepted
            after = rpc('history', conversation=conversation)
            assert after['events'][:len(history['events'])] == history['events']
            assert next(item for item in rpc('conversations') if item['id'] == conversation)['status'] == 'paused'
            log('durable-history-retained', dict(original_event_count=len(history['events']), current_event_count=len(after['events']), paused=True))
            log('reopened-process', subprocess.check_output(['ps', '-p', str(reopened['pid']), '-o', 'pid,ppid,command'], text=True))
            log('reopened-text', subprocess.check_output(['lsof', '-a', '-p', str(reopened['pid']), '-d', 'txt', '-Fn'], text=True))
            rpc('cancel', conversation=conversation, operation_id=str(uuid.uuid4()))
            log('manual-cancel', rpc('conversations'))
            stopped = command([b, 'stop', '--state', state])
            assert stopped.returncode == 0 and 'daemon stopped' in stopped.stdout
            time.sleep(.5)
            assert (state / 'stopped').exists()
            try:
                rpc('identity')
            except (OSError, json.JSONDecodeError):
                log('explicit-stop-preserved', True)
            else:
                raise AssertionError('old frontend restarted after explicit stop')
            offline_failure = command([b, 'activate', '--status', '--request-id', 'fixture-exec-failure', '--state', state])
            assert offline_failure.returncode == 0
            assert json.loads(offline_failure.stdout)['error'] == failed['error']
            # A file restore is deliberately tested after live verification and stop.
            publication('restore', journal)
            assert hashlib.sha256(a.read_bytes()).hexdigest() == hashlib.sha256(args.legacy.read_bytes()).hexdigest()
            log('restore-file-only', json.loads(subprocess.check_output([str(a), '--version', '--json'])))
            log('PASS', dict(loss_phase=args.loss_phase, legacy_continuity_failure=continuity_failure, result='applied + live B at final path; Events loss after Switching; later loss remains B; durable paused operation; manual cancel; explicit stop; conditional restore'))
        finally:
            cleanup_replay(
                release, proxy_stop, proxy,
                lambda: restore_listener(pathlib.Path(sock), upstream,
                                         upstream_identity, proxy_identity),
                daemon_cleanup.close, front, accept_worker, workers, log)


if __name__ == '__main__':
    main()
