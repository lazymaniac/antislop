"""The capability spike must refuse incomplete or misleading observations."""
import copy
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    'probe_restricted_runner', Path(__file__).resolve().parents[1]/'scripts/probe_restricted_runner.py')
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
container_spec = importlib.util.spec_from_file_location(
    'probe_container_runner', Path(__file__).resolve().parents[1]/'scripts/probe_container_runner.py')
container = importlib.util.module_from_spec(container_spec)
container_spec.loader.exec_module(container)


class RestrictedRunnerProbeTests(unittest.TestCase):
    def setUp(self):
        self.observation = {name: {'allowed': False, 'errno': 1}
                            for name in (*probe.REQUIRED_DENIALS, 'child_outside_write', 'child_new_session')}
        self.observation.update(read_selected={'allowed': True, 'correct': True},
                                write_scratch={'allowed': True}, secret_environment_present=False)
        self.controls = dict.fromkeys(probe.REQUIRED_CONTROLS, True)
        self.integrity = dict.fromkeys(probe.REQUIRED_INTEGRITY, True)

    def assess(self, observation=None, controls=None, integrity=None):
        return probe.assess(self.observation if observation is None else observation,
                            self.controls if controls is None else controls,
                            self.integrity if integrity is None else integrity)

    def test_complete_sample_can_pass_without_claiming_autonomous_acceptance(self):
        self.assertEqual([], self.assess())

    def test_each_missing_observation_refuses_the_sample(self):
        for name in self.observation:
            with self.subTest(name=name):
                changed = copy.deepcopy(self.observation)
                del changed[name]
                self.assertTrue(self.assess(observation=changed))

    def test_missing_target_or_no_listener_is_not_proof_of_isolation(self):
        for name in probe.REQUIRED_DENIALS:
            for code in (2, 61, 110):  # ENOENT, ECONNREFUSED, ETIMEDOUT
                with self.subTest(name=name, errno=code):
                    changed = copy.deepcopy(self.observation)
                    changed[name]['errno'] = code
                    self.assertTrue(self.assess(observation=changed))

    def test_child_session_escape_refuses_otherwise_successful_restrictions(self):
        self.observation['child_new_session'] = {'allowed': True}
        self.assertIn('child_new_session_restriction_unproven', self.assess())

    def test_control_omissions_and_source_mutation_refuse_the_sample(self):
        for name in self.controls:
            changed = dict(self.controls)
            del changed[name]
            self.assertTrue(self.assess(controls=changed))
        for name in self.integrity:
            for value in (False, None):
                changed = dict(self.integrity)
                changed[name] = value
                self.assertTrue(self.assess(integrity=changed))

    def test_failed_child_launch_and_inherited_secret_refuse_the_sample(self):
        self.observation['child_outside_write'] = {'inconclusive': True}
        self.assertTrue(self.assess())
        self.setUp()
        self.observation['secret_environment_present'] = True
        self.assertIn('environment_scrubbing_unproven', self.assess())


class ContainerRunnerProbeTests(unittest.TestCase):
    def setUp(self):
        self.source = Path('/owned/selected')
        self.config = {
            'HostConfig': {'PidMode': 'private', 'IpcMode': 'private',
                           'NetworkMode': 'none', 'ReadonlyRootfs': True,
                           'Privileged': False, 'PidsLimit': 32, 'Memory': 268435456,
                           'SecurityOpt': ['no-new-privileges']},
            'Config': {'User': '65534:65534'},
            'Mounts': [{'Type': 'bind', 'Destination': '/input',
                        'Source': str(self.source), 'RW': False}],
        }

    def test_pinned_image_is_required_without_tag_resolution(self):
        for image in ('rust:latest', 'rust:1.98.1', 'sha256:'+'a'*63, '--privileged'):
            with self.subTest(image=image), self.assertRaises(ValueError):
                container.options(image, self.source)
        args = container.options('sha256:'+'a'*64, self.source)
        for flag in ('--pull=never', '--network=none', '--http-proxy=false', '--unsetenv-all'):
            self.assertIn(flag, args)
        with self.assertRaises(ValueError):
            container.options('sha256:'+'a'*64, Path('/owned/extra,dst=/etc'))

    def test_added_host_bind_or_writable_selected_mount_prevents_start(self):
        self.assertTrue(all(container.effective_controls(self.config, self.source).values()))
        self.config['Mounts'].append({'Type': 'bind', 'Destination': '/host', 'Source': '/', 'RW': False})
        self.assertFalse(container.effective_controls(self.config, self.source)['exact_host_binds'])
        self.config['Mounts'].pop()
        self.config['Mounts'][0]['RW'] = True
        self.assertFalse(container.effective_controls(self.config, self.source)['exact_host_binds'])

    def test_weak_runtime_settings_prevent_start(self):
        for name, value in [('NetworkMode', 'host'), ('Privileged', True),
                            ('ReadonlyRootfs', False), ('PidMode', 'host'),
                            ('PidsLimit', 0), ('Memory', 0),
                            ('CapAdd', ['CAP_SYS_ADMIN']), ('SecurityOpt', []),
                            ('Devices', [{'PathOnHost': '/dev/disk0'}])]:
            with self.subTest(setting=name):
                changed = copy.deepcopy(self.config)
                changed['HostConfig'][name] = value
                self.assertFalse(all(container.effective_controls(changed, self.source).values()))

    def test_empty_or_hidden_volume_configurations_are_not_accepted(self):
        self.assertFalse(all(container.effective_controls({}, self.source).values()))
        for extra in [{'Type': 'volume', 'Destination': '/secrets'},
                      {'Type': 'tmpfs', 'Destination': '/input'},
                      {'Type': 'bind', 'Destination': '/input', 'Source': '/other', 'RW': False}]:
            changed = copy.deepcopy(self.config)
            changed['Mounts'].append(extra)
            self.assertFalse(all(container.effective_controls(changed, self.source).values()))

    def test_absent_output_never_establishes_positive_checks_or_network_isolation(self):
        self.assertFalse(all(container.assess({}).values()))
        checks = container.assess({'compile_exit': 0, 'positive_check_exit': 0,
                                   'negative_compile_exit': 0, 'negative_check_exit': 0,
                                   'network_egress': {'allowed': False, 'errno': 111}})
        self.assertTrue(checks['positive_check'])
        self.assertFalse(checks['negative_check'])
        self.assertFalse(checks['network_egress'])
        self.assertFalse(checks['loopback_only'])


if __name__ == '__main__':
    unittest.main()
