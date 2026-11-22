"""Controlled pre-handoff relay failures with a real fixture daemon."""
from pathlib import Path
import os
import socket
import subprocess
import sys
import threading
import time
import unittest
from unittest.mock import Mock
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).parent / 'support'))
from resources import SOURCE, RetainResources, process, resources
from replay_cleanup import cleanup_replay, restore_listener, stop_daemon, RelayConnections


class ReplayCleanupTest(unittest.TestCase):
    def test_failure_after_rename_and_proxy_installation(self):
        binary = SOURCE / 'target/debug/antislop'
        for phase in ('renamed', 'installed'):
            with self.subTest(phase=phase), resources() as (active, diagnostic, env):
                state = active / 'daemon'
                state.mkdir(mode=0o700)
                project = active / 'project'
                project.mkdir()
                config = active / 'config.toml'
                config.write_text((SOURCE / 'examples/antislop.toml').read_text())
                endpoint = state / 'core.sock'
                upstream = state / 'core.sock.upstream'
                with process([str(binary), '__core', str(project), str(config), str(state)],
                             env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as daemon:
                    deadline = time.monotonic() + 10
                    while not endpoint.exists():
                        if daemon.poll() is not None:
                            self.fail(daemon.communicate())
                        self.assertLess(time.monotonic(), deadline, 'daemon readiness')
                        time.sleep(.01)
                    original = endpoint.stat()
                    endpoint.rename(upstream)
                    proxy = None
                    proxy_identity = None
                    if phase == 'installed':
                        proxy = socket.socket(socket.AF_UNIX)
                        proxy.bind(str(endpoint))
                        proxy.listen()
                        proxy_identity = endpoint.stat()
                    release, stopped = threading.Event(), threading.Event()
                    worker = threading.Thread(target=release.wait)
                    worker.start()
                    front = Mock()
                    front.poll.return_value = None
                    front.returncode = -9
                    commands = []
                    def command(argv):
                        result = subprocess.run(list(map(str, argv)), env=env, capture_output=True, text=True)
                        commands.append((result.returncode, result.stdout, result.stderr))
                        return result
                    with self.assertRaisesRegex(RuntimeError, 'injected pre-handoff failure'):
                        try:
                            raise RuntimeError('injected pre-handoff failure')
                        finally:
                            cleanup_replay(release, stopped, proxy,
                                lambda: restore_listener(endpoint, upstream, original, proxy_identity),
                                lambda: stop_daemon(command, binary, state), front, None, [worker],
                                lambda label, value: (diagnostic / 'cleanup.log').write_text(str(value)))
                    daemon.wait(timeout=10)
                    self.assertEqual(daemon.returncode, 0)
                    self.assertFalse(worker.is_alive())
                    self.assertFalse(upstream.exists())
                    self.assertEqual(commands[0][0], 0)
                    self.assertIn('daemon stopped', commands[0][1])
                saved_active = active
            self.assertFalse(saved_active.exists())

    def test_cleanup_continues_after_wait_and_stop_errors(self):
        release, stopped = threading.Event(), threading.Event()
        worker = threading.Thread(target=release.wait)
        worker.start()
        front = Mock()
        front.poll.return_value = None
        front.wait.side_effect = OSError('injected wait failure')
        restore = Mock()
        stop = Mock(side_effect=RuntimeError('injected stop failure'))
        with self.assertRaisesRegex(RetainResources, 'injected wait failure.*injected stop failure'):
            cleanup_replay(release, stopped, None, restore, stop, front, None, [worker], Mock())
        restore.assert_called_once()
        stop.assert_called_once()
        self.assertFalse(worker.is_alive())

    def test_stop_failure_unblocks_active_relay_reads(self):
        release, stopped = threading.Event(), threading.Event()
        for buffered in (False, True):
            with self.subTest(buffered=buffered):
                frontend, backend = socket.socketpair()
                entered, finished = threading.Event(), threading.Event()
                def relay():
                    try:
                        with frontend:
                            entered.set()
                            if buffered:
                                with frontend.makefile('rb') as stream:
                                    stream.readline()
                            else:
                                frontend.recv(1)
                    except OSError:
                        pass
                    finally:
                        connections.remove(frontend)
                        finished.set()
                connections = RelayConnections()
                connections.add(frontend)
                worker = threading.Thread(target=relay)
                worker.start()
                entered.wait()
                front = Mock()
                front.poll.return_value = 0
                front.returncode = 0
                try:
                    with self.assertRaisesRegex(RetainResources, 'injected stop failure'):
                        cleanup_replay(release, stopped, None, lambda: None,
                            Mock(side_effect=RuntimeError('injected stop failure')),
                            front, None, [worker], Mock(), connections)
                    self.assertTrue(finished.is_set())
                    self.assertFalse(worker.is_alive())
                    late, peer = socket.socketpair()
                    with peer:
                        self.assertFalse(connections.add(late))
                        self.assertEqual(late.fileno(), -1)
                finally:
                    backend.close()
                    worker.join()

    def test_false_stop_success_retains_resources_and_diagnostics(self):
        import fcntl
        import shutil
        active = None
        try:
            with self.assertRaisesRegex(RetainResources, 'daemon still owns'):
                with resources() as (active, diagnostic, _):
                    state = active / 'daemon'
                    state.mkdir()
                    with (state / 'core.lock').open('wb') as lock:
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        result = subprocess.CompletedProcess([], 0, 'daemon is not running', '')
                        stop_daemon(lambda argv: result, 'fixture-binary', state)
            self.assertTrue(active.exists())
            self.assertIn('daemon still owns', (diagnostic / 'exception.log').read_text())
        finally:
            if active is not None:
                shutil.rmtree(active)

    def test_successor_endpoint_is_preserved(self):
        with resources() as (active, _, _):
            endpoint, upstream = active / 's', active / 'u'
            with socket.socket(socket.AF_UNIX) as old, socket.socket(socket.AF_UNIX) as successor:
                old.bind(str(endpoint))
                original = endpoint.stat()
                endpoint.rename(upstream)
                successor.bind(str(endpoint))
                successor_identity = endpoint.stat()
                restore_listener(endpoint, upstream, original, None)
                self.assertEqual(endpoint.stat().st_ino, successor_identity.st_ino)


if __name__ == '__main__':
    unittest.main()
