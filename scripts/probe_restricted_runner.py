#!/usr/bin/env python3
"""Opt-in §13 macOS sandbox spike. No provider, project or user configuration access.

Runs only owned disposable programs/data. A successful probe is not approval of
an autonomous runner: process containment and other untested guarantees remain
explicit. Nonzero exit means unavailable, inconclusive or a restriction failed.
"""
import errno
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import socket
import subprocess
import tempfile


PROFILE = '''(version 1)
(deny default)
(allow process-exec)
(allow process-fork)
(allow process-info* (target same-sandbox))
(allow signal (target same-sandbox))
(deny process-info-setcontrol)
(allow file-read-metadata (vnode-type DIRECTORY))
(allow file-read* (literal "/")
    (subpath "/System/Library") (subpath "/usr/bin")
    (subpath "/usr/lib") (subpath "/usr/share")
    (subpath "/bin") (subpath "/sbin")
    (subpath "/Library/Developer/CommandLineTools")
    (subpath (param "CHECK_PROJECT")) (subpath (param "CHECK_SCRATCH")))
(allow file-read* (literal "/dev/null") (literal "/dev/zero")
    (literal "/dev/random") (literal "/dev/urandom"))
(allow file-write-data (literal "/dev/null"))
(allow file-write* (subpath (param "CHECK_SCRATCH")))
(allow sysctl-read (sysctl-name "hw.ncpu") (sysctl-name "hw.activecpu")
    (sysctl-name "hw.pagesize") (sysctl-name "kern.osrelease")
    (sysctl-name "kern.osversion") (sysctl-name "kern.ostype")
    (sysctl-name "kern.argmax"))
'''

# No input from a repository or a model is executed. Output contains booleans,
# errno/status only; neither sentinel content nor native/user secrets are logged.
PROGRAM = r'''
import json,os,pathlib,socket,subprocess,sys
project,scratch,protected=map(pathlib.Path,sys.argv[1:4]);port=int(sys.argv[4]);r={}
def attempt(name,fn):
 try:
  value=fn();r[name]={'allowed':True,'correct':value if isinstance(value,bool) else None}
 except OSError as e:r[name]={'allowed':False,'errno':e.errno}
attempt('read_selected',lambda:(project/'input').read_bytes()==b'selected source')
attempt('write_scratch',lambda:(scratch/'output').write_text('generated output'))
attempt('read_protected',lambda:(protected/'secret').read_bytes())
attempt('write_selected',lambda:(project/'input').write_bytes(b'changed'))
attempt('write_protected',lambda:(protected/'secret').write_bytes(b'changed'))
attempt('symlink_read_protected',lambda:(project/'outside-link').read_bytes())
attempt('symlink_write_protected',lambda:(scratch/'write-link').write_bytes(b'changed'))
def hardlink():
 os.link(project/'input',scratch/'hardlink')
 (scratch/'hardlink').write_bytes(b'changed')
attempt('hardlink_write_selected',hardlink)
def network():
 with socket.socket() as s:
  s.settimeout(1);s.connect(('127.0.0.1',port))
attempt('network_loopback',network)
def unix_connect():
 with socket.socket(socket.AF_UNIX) as s:
  s.settimeout(1);s.connect(str(protected/'socket'))
attempt('unix_socket',unix_connect)
for name,action in [('child_new_session','os.setsid()'),
                    ('child_outside_write',"pathlib.Path(sys.argv[1]).write_text('changed')")]:
 code="import json,os,pathlib,sys\ntry:\n "+action+"\n print(json.dumps({'allowed':True}))\nexcept OSError as e:\n print(json.dumps({'allowed':False,'errno':e.errno}))"
 child=subprocess.run([sys.executable,'-c',code,str(protected/'child')],
  stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
 r[name]=json.loads(child.stdout) if child.returncode==0 else {'inconclusive':True}
r['secret_environment_present']='ANTISLOP_DISPOSABLE_SECRET' in os.environ
print(json.dumps(r))
'''

REQUIRED_DENIALS = (
    'read_protected', 'write_selected', 'write_protected',
    'symlink_read_protected', 'symlink_write_protected',
    'hardlink_write_selected', 'network_loopback', 'unix_socket',
)
REQUIRED_CONTROLS = ('tcp_connect', 'unix_connect', 'protected_read', 'symlink_write', 'hardlink_write')
REQUIRED_INTEGRITY = ('selected_unchanged', 'protected_unchanged', 'protected_child_absent')


def assess(observation, controls, integrity):
    """Missing observations and unrelated syscall errors never prove a denial."""
    failures = []
    if not all(controls.get(name) is True for name in REQUIRED_CONTROLS):
        failures.append('unsandboxed_controls_incomplete')
    if observation.get('read_selected') != {'allowed': True, 'correct': True}:
        failures.append('selected_read_unproven')
    if observation.get('write_scratch', {}).get('allowed') is not True:
        failures.append('scratch_write_unproven')
    for name in (*REQUIRED_DENIALS, 'child_outside_write', 'child_new_session'):
        item = observation.get(name, {})
        if item.get('allowed') is not False or item.get('errno') not in (errno.EPERM, errno.EACCES):
            failures.append(name + '_restriction_unproven')
    if observation.get('secret_environment_present') is not False:
        failures.append('environment_scrubbing_unproven')
    if not all(integrity.get(name) is True for name in REQUIRED_INTEGRITY):
        failures.append('fixture_integrity_failed')
    return failures


def run():
    report = {
        'schema': 'restricted-runner-spike-v1', 'host': platform.system(),
        'host_release': platform.release(), 'backend': 'macos-seatbelt-spike',
        'autonomous_ready': False, 'status': 'unavailable',
        'profile_sha256': hashlib.sha256(PROFILE.encode()).hexdigest(),
        'untested': ['linux', 'resource_limits', 'hostile_descendant_cleanup',
                     'toolchain_compatibility', 'system_read_root_audit',
                     'runtime_authorization_and_evidence_binding'],
    }
    python = Path('/Library/Developer/CommandLineTools/usr/bin/python3')
    if platform.system() != 'Darwin' or not Path('/usr/bin/sandbox-exec').is_file() or not python.is_file():
        report['reason'] = 'this spike requires macOS sandbox-exec and CommandLineTools Python'
        return report
    # A short canonical path is necessary for macOS AF_UNIX's pathname limit.
    # Spaces, brackets, quotes and backslashes also test literal SBPL parameters.
    with tempfile.TemporaryDirectory(prefix='as [x] "\\', dir='/tmp') as temporary:
        root = Path(temporary).resolve()
        project, scratch, protected = [root / name for name in ('project', 'scratch', 'protected')]
        for directory in (project, scratch, protected):
            directory.mkdir(mode=0o700)
        selected, secret = project/'input', protected/'secret'
        selected.write_bytes(b'selected source')
        secret.write_bytes(b'disposable sentinel')
        (project/'outside-link').symlink_to(secret)
        (scratch/'write-link').symlink_to(secret)
        (project/'probe.py').write_text(PROGRAM)
        profile = root/'profile.sb'
        profile.write_text(PROFILE)
        with socket.socket() as tcp, socket.socket(socket.AF_UNIX) as unix:
            tcp.bind(('127.0.0.1', 0)); tcp.listen(4)
            unix.bind(str(protected/'socket')); unix.listen(4)
            port = tcp.getsockname()[1]
            # Positive controls prove the disposable targets exist and ordinary
            # ambient permissions/network would allow the attempted actions.
            with socket.create_connection(('127.0.0.1', port), timeout=1):
                pass
            connection, _ = tcp.accept(); connection.close()
            with socket.socket(socket.AF_UNIX) as control:
                control.connect(str(protected/'socket'))
            connection, _ = unix.accept(); connection.close()
            controls = {'tcp_connect': True, 'unix_connect': True,
                        'protected_read': secret.read_bytes() == b'disposable sentinel'}
            (scratch/'write-link').write_bytes(b'control')
            controls['symlink_write'] = secret.read_bytes() == b'control'
            secret.write_bytes(b'disposable sentinel')
            os.link(selected, scratch/'control-hardlink')
            (scratch/'control-hardlink').write_bytes(b'control')
            controls['hardlink_write'] = selected.read_bytes() == b'control'
            (scratch/'control-hardlink').unlink()
            selected.write_bytes(b'selected source')
            environment = {'PATH': '/usr/bin:/bin', 'HOME': str(scratch),
                           'TMPDIR': str(scratch), 'PYTHONDONTWRITEBYTECODE': '1',
                           'PYTHONNOUSERSITE': '1', 'LANG': 'C.UTF-8'}
            argv = ['/usr/bin/sandbox-exec', '-D', f'CHECK_PROJECT={project}',
                    '-D', f'CHECK_SCRATCH={scratch}', '-f', str(profile),
                    str(python), str(project/'probe.py'), str(project),
                    str(scratch), str(protected), str(port)]
            process = subprocess.Popen(argv, cwd=project, env=environment,
                                       stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                       stderr=subprocess.DEVNULL, start_new_session=True)
            try:
                stdout, _ = process.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                # The leader has not been reaped; its PID still anchors the
                # group. Never signal a stored process-group ID after wait().
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                report.update(status='inconclusive', reason='owned probe timed out')
                return report
            finally:
                # These fixed programs do not leave escaped descendants alive.
                # Cleanup of arbitrary hostile descendants remains unproven.
                process.wait()
            report['exit_code'] = process.returncode
            if process.returncode != 0 or len(stdout) > 16384:
                report.update(status='inconclusive', reason='owned probe did not return bounded observations')
                return report
            try:
                observation = json.loads(stdout)
                if not isinstance(observation, dict):
                    raise ValueError()
            except (ValueError, UnicodeError):
                report.update(status='inconclusive', reason='owned probe returned invalid observations')
                return report
            integrity = {
                'selected_unchanged': selected.read_bytes() == b'selected source',
                'protected_unchanged': secret.read_bytes() == b'disposable sentinel',
                'protected_child_absent': not (protected/'child').exists(),
            }
            failures = assess(observation, controls, integrity)
            report.update(status='rejected' if failures else 'sampled_restrictions_passed',
                          observations=observation, controls=controls, integrity=integrity,
                          failures=failures)
    return report


def main():
    try:
        report = run()
    except (OSError, subprocess.SubprocessError):
        # Do not include arbitrary subprocess diagnostics or environmental paths.
        report = {'schema': 'restricted-runner-spike-v1', 'status': 'inconclusive',
                  'autonomous_ready': False, 'reason': 'owned setup or execution failed'}
    print(json.dumps(report, indent=2))
    return 0 if report['status'] == 'sampled_restrictions_passed' else 2


if __name__ == '__main__':
    raise SystemExit(main())
