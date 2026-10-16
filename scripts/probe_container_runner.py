#!/usr/bin/env python3
"""Opt-in, provider-free container spike using an explicitly pinned local image.

Only generated fixture programs execute. No image pull, VM start, user project
mount, native login or runtime-policy update. Not an autonomous check runner.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import time
import uuid


IMAGE_ID = re.compile(r'sha256:[0-9a-f]{64}\Z')
CONTAINER_ID = re.compile(r'[0-9a-f]{64}\Z')
LABEL = 'io.antislop.runner-spike'
SOURCE = b'pub fn sum(a:i32,b:i32)->i32{a+b}\n#[test]fn sum_works(){assert_eq!(sum(2,3),5)}\n'
PROGRAM = r'''
import json,os,pathlib,socket,subprocess,sys
r={};source=pathlib.Path('/input/lib.rs')
def attempt(name,fn):
 try:
  value=fn();r[name]={'allowed':True,'correct':value if isinstance(value,bool) else None}
 except OSError as e:r[name]={'allowed':False,'errno':e.errno}
attempt('read_selected',lambda:source.read_bytes().startswith(b'pub fn sum'))
attempt('write_scratch',lambda:pathlib.Path('/scratch/output').write_text('owned'))
attempt('write_selected',lambda:source.write_text('changed'))
attempt('write_root',lambda:pathlib.Path('/usr/antislop-probe').write_text('changed'))
attempt('read_host_sentinel',lambda:pathlib.Path(sys.argv[1]).read_bytes())
attempt('symlink_read_host',lambda:pathlib.Path('/input/outside-link').read_bytes())
attempt('host_engine_socket',lambda:pathlib.Path('/run/podman/podman.sock').open('rb'))
def hardlink():
 os.link(source,'/scratch/alias');pathlib.Path('/scratch/alias').write_text('changed')
attempt('hardlink_write_selected',hardlink)
def network():
 with socket.socket() as s:
  s.settimeout(1);s.connect(('192.0.2.1',9))
attempt('network_egress',network)
r['network_interfaces']=sorted(os.listdir('/sys/class/net'))
r['uid']=os.getuid()
r['secret_environment_present']='ANTISLOP_DISPOSABLE_SECRET' in os.environ
status=pathlib.Path('/proc/self/status').read_text().splitlines()
r['process_controls']={line.split(':',1)[0]:line.split(':',1)[1].strip()
 for line in status if line.startswith(('CapEff:','NoNewPrivs:','Seccomp:'))}
r['cgroup_limits']={name:pathlib.Path('/sys/fs/cgroup',name).read_text().strip()
 for name in ('memory.max','pids.max','cpu.max')}
def execute(argv):
 return subprocess.run(argv,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=15).returncode
r['compile_exit']=execute(['rustc','--test','-D','warnings','/input/lib.rs','-o','/scratch/checks'])
r['positive_check_exit']=execute(['/scratch/checks']) if r['compile_exit']==0 else None
pathlib.Path('/scratch/failing.rs').write_text('#[test]fn fails(){assert_eq!(1,2);}')
r['negative_compile_exit']=execute(['rustc','--test','/scratch/failing.rs','-o','/scratch/failing'])
r['negative_check_exit']=execute(['/scratch/failing']) if r['negative_compile_exit']==0 else None
print(json.dumps(r))
'''

# The only host-writable mount in this second fixture is its own observation
# directory. A new-session child emits a heartbeat until PID-namespace teardown.
CLEANUP_PROGRAM = r'''
import json,os,pathlib,subprocess,sys,time
child=subprocess.Popen([sys.executable,'-c',
 "import os,pathlib,time;os.setsid();p=pathlib.Path('/observations/heartbeat');p.write_text(str(os.getpid())+'\\n');\nwhile True:\n with p.open('a') as f:f.write('.')\n time.sleep(.05)"],
 stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
deadline=time.monotonic()+3
while not pathlib.Path('/observations/heartbeat').exists():
 if time.monotonic()>deadline:raise RuntimeError('no child heartbeat')
 time.sleep(.01)
print(json.dumps({'child':child.pid,'child_session':os.getsid(child.pid),
 'parent_session':os.getsid(0),'child_group':os.getpgid(child.pid),'parent_group':os.getpgrp()}),flush=True)
time.sleep(30)
'''


def options(image, source, observations=None):
    if not IMAGE_ID.fullmatch(image):
        raise ValueError('an exact local sha256 image ID is required')
    if any(',' in str(path) for path in (source, observations) if path is not None):
        raise ValueError('comma in mount path is unsupported by this spike')
    result = ['--pull=never', '--network=none', '--read-only', '--read-only-tmpfs=false',
              '--cap-drop=all', '--security-opt=no-new-privileges', '--pid=private',
              '--ipc=private', '--cgroupns=private', '--user=65534:65534',
              '--pids-limit=32', '--memory=256m', '--cpus=1', '--timeout=40',
              '--http-proxy=false', '--unsetenv-all', '--image-volume=ignore',
              '--tmpfs=/scratch:rw,exec,nosuid,nodev,size=32m,mode=1777',
              '--env=HOME=/scratch', '--env=TMPDIR=/scratch',
              '--env=PATH=/usr/local/cargo/bin:/usr/bin:/bin',
              '--env=RUSTUP_HOME=/usr/local/rustup',
              '--workdir=/input',
              f'--mount=type=bind,src={source},dst=/input,ro,nosuid,nodev',
              '--entrypoint=/usr/bin/python3']
    if observations is not None:
        result.append(f'--mount=type=bind,src={observations},dst=/observations,rw,nosuid,nodev')
    return result


def effective_controls(config, source, observations=None):
    """Inspect before start; defaults must not add another host bind mount."""
    host = config.get('HostConfig', {})
    mounts = config.get('Mounts', [])
    expected = {'/input': (str(source), False)}
    if observations is not None:
        expected['/observations'] = (str(observations), True)
    actual = {m.get('Destination'): (m.get('Source'), m.get('RW'))
              for m in mounts if m.get('Type') == 'bind'}
    return {
        'exact_host_binds': actual == expected and len(actual) == sum(m.get('Type') == 'bind' for m in mounts),
        'no_extra_volumes': all(m.get('Type') == 'bind' or
                                (m.get('Type') == 'tmpfs' and m.get('Destination') == '/scratch')
                                for m in mounts),
        'private_pid': host.get('PidMode') == 'private',
        'private_ipc': host.get('IpcMode') == 'private',
        'no_network': host.get('NetworkMode') == 'none',
        'read_only_root': host.get('ReadonlyRootfs') is True,
        'unprivileged': host.get('Privileged') is False,
        'fixed_user': config.get('Config', {}).get('User') == '65534:65534',
        'bounded_pids': host.get('PidsLimit') == 32,
        'bounded_memory': host.get('Memory') == 256 * 1024 * 1024,
        'no_added_caps': not host.get('CapAdd'),
        'no_new_privileges': 'no-new-privileges' in host.get('SecurityOpt', []),
        'no_host_devices': not host.get('Devices'),
        'no_published_ports': not host.get('PortBindings'),
    }


def assess(observation):
    controls = {
        'selected_read': observation.get('read_selected') == {'allowed': True, 'correct': True},
        'scratch_write': observation.get('write_scratch', {}).get('allowed') is True,
        'loopback_only': observation.get('network_interfaces') == ['lo'],
        'fixed_uid': observation.get('uid') == 65534,
        'scrubbed_environment': observation.get('secret_environment_present') is False,
        'kernel_controls': observation.get('process_controls') == {
            'CapEff': '0000000000000000', 'NoNewPrivs': '1', 'Seccomp': '2'},
        'effective_resource_limits': observation.get('cgroup_limits') == {
            'memory.max': '268435456', 'pids.max': '32', 'cpu.max': '100000 100000'},
        'positive_check': observation.get('compile_exit') == 0 and observation.get('positive_check_exit') == 0,
        'negative_check': observation.get('negative_compile_exit') == 0 and observation.get('negative_check_exit') == 101,
    }
    # ENOENT is expected only for unmounted host paths/sockets, never evidence
    # that a mounted source is read-only or that the network namespace works.
    for name, codes in {
        'write_selected': (30,), 'write_root': (30,),
        'read_host_sentinel': (2,), 'symlink_read_host': (2,),
        'host_engine_socket': (2,), 'hardlink_write_selected': (18, 30, 1),
        'network_egress': (101,),
    }.items():
        item = observation.get(name, {})
        controls[name] = item.get('allowed') is False and item.get('errno') in codes
    return controls


class Engine:
    def __init__(self, binary):
        self.binary = binary
        self.token = str(uuid.uuid4())
        self.owned = []

    def call(self, *args, timeout=20):
        result = subprocess.run([self.binary, *args], stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=timeout)
        if result.returncode or len(result.stdout) > 128 * 1024:
            raise RuntimeError('owned engine operation failed')
        return result.stdout

    def inspect(self, container):
        records = json.loads(self.call('inspect', container))
        if not isinstance(records, list) or len(records) != 1 or not isinstance(records[0], dict):
            raise RuntimeError('invalid owned container inspection')
        config = records[0]
        if config.get('Config', {}).get('Labels', {}).get(LABEL) != self.token:
            raise RuntimeError('container ownership mismatch')
        if not CONTAINER_ID.fullmatch(config.get('Id', '')):
            raise RuntimeError('invalid inspected container identity')
        return config

    def create(self, image, source, observations=None, args=()):
        # Register the unpredictable owned name before create, so a timed-out
        # create is still discoverable for label-checked cleanup. Never retry it.
        name = 'antislop-runner-spike-' + str(uuid.uuid4())
        self.owned.append(name)
        container = self.call('create', '--name', name, '--label', f'{LABEL}={self.token}',
                              *options(image, source, observations), image, *args).decode().strip()
        if not CONTAINER_ID.fullmatch(container):
            raise RuntimeError('invalid owned container ID')
        config = self.inspect(container)
        checks = effective_controls(config, source, observations)
        if not all(checks.values()):
            raise RuntimeError('effective container restrictions differ from requested fixture')
        return container, checks

    def cleanup(self):
        clean = True
        for name in self.owned:
            try:
                config = self.inspect(name)
                self.call('rm', '--force', config['Id'])
            except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
                clean = False
        return clean


def run(binary, image):
    engine = Engine(binary)
    report = {'schema': 'container-runner-spike-v1', 'image_id': image,
              'host': platform.system(), 'host_release': platform.release(),
              'generated_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
              'autonomous_ready': False, 'status': 'inconclusive',
              'fixture_program_sha256': hashlib.sha256(PROGRAM.encode()).hexdigest(),
              'cleanup_program_sha256': hashlib.sha256(CLEANUP_PROGRAM.encode()).hexdigest(),
              'untested': ['daemon_death_recovery', 'runtime_policy_and_evidence_binding',
                           'arbitrary_project_toolchains', 'native_linux_host',
                           'engine_configuration_trust', 'secret_selection_and_retention',
                           'resource_exhaustion_behaviour']}
    try:
        version = json.loads(engine.call('version', '--format', 'json'))
        report['engine_versions'] = {key: version.get(key, {}).get('Version') for key in ('Client', 'Server')}
        # Inspect only the explicitly requested local image; never resolve/pull a tag.
        actual = engine.call('image', 'inspect', image, '--format', '{{.Id}}').decode().strip()
        if actual.removeprefix('sha256:') != image.removeprefix('sha256:'):
            raise RuntimeError('image identity mismatch')
        with tempfile.TemporaryDirectory(prefix='antislop-container-', dir='/tmp') as temporary:
            root = Path(temporary).resolve()
            source, observed = root/'input', root/'observations'
            source.mkdir(mode=0o755); observed.mkdir(mode=0o777)
            observed.chmod(0o777)  # Only this owned heartbeat fixture needs a host-writable bind.
            (root/'secret').write_bytes(b'disposable host sentinel')
            (source/'lib.rs').write_bytes(SOURCE)
            (source/'probe.py').write_text(PROGRAM)
            (source/'cleanup.py').write_text(CLEANUP_PROGRAM)
            for path in source.iterdir():
                path.chmod(0o644)
            (source/'outside-link').symlink_to(root/'secret')
            container, checks = engine.create(image, source, args=('/input/probe.py', str(root/'secret')))
            stdout = engine.call('start', '--attach', container, timeout=30)
            observation = json.loads(stdout)
            state = engine.inspect(container)['State']
            checks.update(assess(observation))
            checks['check_container_exited_cleanly'] = state.get('Running') is False and state.get('ExitCode') == 0
            checks['selected_source_unchanged'] = (source/'lib.rs').read_bytes() == SOURCE
            checks['host_sentinel_unchanged'] = (root/'secret').read_bytes() == b'disposable host sentinel'
            cleanup, cleanup_checks = engine.create(image, source, observed, args=('/input/cleanup.py',))
            engine.call('start', cleanup)
            deadline = time.monotonic()+5
            child = None
            while time.monotonic() < deadline:
                logs = engine.call('logs', cleanup).strip()
                if logs:
                    child = json.loads(logs)
                    break
                time.sleep(.05)
            if child is None:
                raise RuntimeError('new-session child was not observed')
            heartbeat = observed/'heartbeat'
            before = heartbeat.stat().st_size
            time.sleep(.2)
            cleanup_checks['child_heartbeat_live'] = heartbeat.stat().st_size > before
            cleanup_checks['child_changed_session'] = child['child_session'] != child['parent_session']
            cleanup_checks['child_changed_group'] = child['child_group'] != child['parent_group']
            engine.call('kill', '--signal=KILL', cleanup)
            waited = engine.call('wait', cleanup).decode().strip()
            state = engine.inspect(cleanup)['State']
            after = heartbeat.stat().st_size
            time.sleep(.3)
            cleanup_checks['child_heartbeat_stopped'] = heartbeat.stat().st_size == after
            cleanup_checks['container_stopped'] = state.get('Running') is False and state.get('Pid') == 0 and waited == '137'
            report.update(status='sampled_restrictions_passed' if all(checks.values()) and all(cleanup_checks.values()) else 'rejected',
                          checks=checks, cleanup_checks=cleanup_checks, observations=observation)
    except (RuntimeError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
        report['reason'] = 'owned setup, restriction check or execution failed; no provider or project command was launched'
    finally:
        report['owned_container_names'] = list(engine.owned)
        report['owned_containers_removed'] = engine.cleanup()
        if not report['owned_containers_removed']:
            report['status'] = 'inconclusive'
            report['reason'] = 'owned container cleanup is unconfirmed'
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True, help='exact already-local sha256 image ID with Python3 and rustc')
    parser.add_argument('--podman', default=shutil.which('podman'))
    args = parser.parse_args()
    if not IMAGE_ID.fullmatch(args.image) or not args.podman:
        parser.error('requires a Podman binary and an exact local sha256 image ID')
    report = run(args.podman, args.image)
    print(json.dumps(report, indent=2))
    return 0 if report['status'] == 'sampled_restrictions_passed' else 2


if __name__ == '__main__':
    raise SystemExit(main())
