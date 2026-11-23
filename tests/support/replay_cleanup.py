"""Cleanup for the exclusively owned bootstrap replay fixture."""
import fcntl
import os
import socket
import threading

from resources import RetainResources


class RelayConnections:
    """Own active relay sockets and unblock their reads before joining workers."""
    def __init__(self):
        self.lock = threading.Lock()
        self.sockets = set()
        self.stopped = False

    def add(self, connection):
        with self.lock:
            if self.stopped:
                connection.close()
                return False
            self.sockets.add(connection)
            return True

    def remove(self, connection):
        with self.lock:
            self.sockets.discard(connection)

    def shutdown(self):
        with self.lock:
            self.stopped = True
            for connection in self.sockets:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass  # Already closed or not connected.
                connection.close()


def identity(path):
    try:
        value = path.stat()
    except FileNotFoundError:
        return None
    return value.st_dev, value.st_ino


def restore_listener(endpoint, upstream, original, proxy):
    """Restore only our renamed listener; preserve a successor's rebound socket."""
    if original is None:
        return
    original = original.st_dev, original.st_ino
    if identity(upstream) != original:
        return
    current = identity(endpoint)
    if proxy is not None and current == (proxy.st_dev, proxy.st_ino):
        endpoint.unlink()
        current = None
    if current is None:
        os.rename(upstream, endpoint)
    # A different inode belongs to the successor and already has the right name.


def stop_daemon(command, binary, state):
    result = command([binary, 'stop', '--state', state, '--force'])
    if result.returncode != 0:
        raise RetainResources(f'daemon stop failed: {result.returncode}: {result.stdout} {result.stderr}')
    lock = state / 'core.lock'
    if lock.exists():
        with lock.open('rb') as stream:
            try:
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise RetainResources(f'daemon still owns {lock}: {result.stdout} {result.stderr}') from error


def cleanup_replay(release, proxy_stop, proxy, restore, stop, front, accept_worker, workers, log, connections=None):
    """Attempt every release even if another fails; retain state on any failure."""
    errors = []

    def attempt(action):
        try:
            action()
        except Exception as error:
            errors.append(error)

    release.set()
    proxy_stop.set()
    if proxy is not None:
        attempt(proxy.close)
    # Prevent the frontend from autostarting another daemon during shutdown.
    if front.poll() is None:
        attempt(front.kill)
    attempt(front.wait)
    attempt(restore)
    attempt(stop)
    if connections is not None:
        attempt(connections.shutdown)
    if accept_worker is not None:
        attempt(accept_worker.join)
    for worker in workers:
        attempt(worker.join)
    attempt(lambda: log('cleanup', dict(frontend_exit=front.returncode,
                                      errors=[str(error) for error in errors])))
    if errors:
        raise RetainResources('replay cleanup failed: ' + '; '.join(map(str, errors))) from errors[0]
