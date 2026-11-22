"""Resource contract tests; run with python3 -B tests/test_resources.py."""
from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).parent / 'support'))
from resources import SOURCE, external, process, resources


class ResourcesTest(unittest.TestCase):
    def test_parallel_cleanup_and_failure_diagnostics(self):
        def run(_):
            with resources() as (active, diagnostic, env):
                self.assertLess(len(os.fsencode(active / 's/core.sock')), 100)
                (active / 'raw.log').write_text('raw failure')
                self.assertNotEqual(env['ANTISLOP_CONFIG_DIR'], os.environ.get('ANTISLOP_CONFIG_DIR'))
            self.assertFalse(active.exists())
            self.assertEqual((diagnostic / 'remaining/raw.log').read_text(), 'raw failure')
            return active
        with ThreadPoolExecutor(max_workers=8) as pool:
            self.assertEqual(len(set(pool.map(run, range(16)))), 16)

    def test_error_reaps_process_before_directory_cleanup(self):
        child = None
        with self.assertRaisesRegex(RuntimeError, 'readiness failed'):
            with resources() as (active, diagnostic, env):
                (active / 'failure.stderr').write_text('readiness failed')
                with process([sys.executable, '-c', 'import sys; sys.stdin.read()'],
                             stdin=-1, env=env) as child:
                    raise RuntimeError('readiness failed')
        self.assertIsNotNone(child.poll())
        self.assertFalse(active.exists())
        self.assertEqual((diagnostic / 'remaining/failure.stderr').read_text(), 'readiness failed')

    def test_source_alias_and_long_root_are_rejected_before_writes(self):
        for name in ('ANTISLOP_TEST_TEMP_ROOT', 'ANTISLOP_TEST_DIAGNOSTICS'):
            with patch.dict(os.environ, {name: ''}):
                with self.assertRaisesRegex(ValueError, 'empty'):
                    with resources():
                        self.fail('empty root accepted')
        with self.assertRaises(ValueError):
            external(SOURCE / 'must-not-be-created', strict=False)
        with resources() as (active, _, _):
            alias = active / 'alias'
            alias.symlink_to(SOURCE, target_is_directory=True)
            with self.assertRaises(ValueError):
                external(alias)
            long_root = active / ('long' * 30)
            long_root.mkdir()
            with patch.dict(os.environ, ANTISLOP_TEST_TEMP_ROOT=str(long_root)):
                with self.assertRaisesRegex(ValueError, 'shorter'):
                    with resources():
                        self.fail('long root accepted')
        self.assertFalse((SOURCE / 'must-not-be-created').exists())

    def test_setup_and_diagnostic_failures_keep_an_actionable_result(self):
        import shutil
        import tempfile
        allocations = []
        original_allocate = tempfile.mkdtemp
        original_mkdir = Path.mkdir
        def allocate(**kwargs):
            path = original_allocate(**kwargs)
            allocations.append(Path(path))
            return path
        def mkdir(path, *args, **kwargs):
            if path.name == 'config':
                raise OSError('fixture mkdir failed')
            return original_mkdir(path, *args, **kwargs)
        with patch('resources.Path.mkdir', new=mkdir), patch('resources.tempfile.mkdtemp', side_effect=allocate):
            with self.assertRaisesRegex(OSError, 'fixture mkdir failed'):
                with resources():
                    self.fail('setup failure ignored')
        self.assertFalse(allocations[-1].exists())
        self.assertIn('fixture mkdir failed', (allocations[-2] / 'exception.log').read_text())
        active = None
        with patch('resources.preserve_logs', side_effect=OSError('diagnostic copy failed')):
            with self.assertRaisesRegex(OSError, 'diagnostic copy failed'):
                with resources() as (active, diagnostic, _):
                    (active / 'failure.log').write_text('retain me')
        self.assertTrue(active.exists())
        self.assertEqual((active / 'failure.log').read_text(), 'retain me')
        # This test, and only this test, owns this exact retained directory.
        shutil.rmtree(active)

    def test_runner_sets_environment_before_start_and_cleans_after_nonzero_exit(self):
        import json
        import subprocess
        with resources() as (_, diagnostic, env):
            env['ANTISLOP_TEST_EXTERNAL_CWD'] = '1'
            code = "import json,os; print(json.dumps(dict(cwd=os.getcwd(),root=os.environ['ANTISLOP_TEST_ROOT'],config=os.environ['ANTISLOP_CONFIG_DIR']))); raise SystemExit(7)"
            cmd = [sys.executable, '-B', str(SOURCE / 'tests/support/run_test.py'), sys.executable, '-c', code]
            result = subprocess.run(cmd, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 7, result.stderr)
            values = json.loads(result.stdout)
            self.assertEqual(values['cwd'], values['root'])
            self.assertTrue(Path(values['config']).is_relative_to(values['root']))
            self.assertFalse(Path(values['root']).exists())
            child_diagnostic = next(diagnostic.glob('run-*'))
            self.assertIn('exit=7', (child_diagnostic / 'command.log').read_text())
            self.assertIn(values['root'], (child_diagnostic / 'output.log').read_text())

    def test_child_retention_marker_preserves_launcher_root_and_diagnostics(self):
        import shutil
        with resources() as (active, diagnostic, _):
            (active / 'retain-resources.log').write_text('daemon lock not released')
        try:
            self.assertTrue(active.exists())
            self.assertEqual((diagnostic / 'remaining/retain-resources.log').read_text(),
                             'daemon lock not released')
        finally:
            shutil.rmtree(active)

    def test_missing_child_cleanup(self):
        with resources() as (active, _, env):
            with self.assertRaises(FileNotFoundError):
                with process([str(active / 'missing')], env=env):
                    self.fail('missing command started')
        self.assertFalse(active.exists())


if __name__ == '__main__':
    unittest.main()
