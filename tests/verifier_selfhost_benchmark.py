"""Independent offline benchmark counterexamples; no real provider or source changes."""
import importlib.util
import json
import ast
import hashlib
import math
from pathlib import Path
import subprocess
import sys
import statistics
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[1] / 'scripts/selfhost_benchmark.py'
spec = importlib.util.spec_from_file_location('independent_benchmark', MODULE)
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class Fixture:
    def __init__(self, root, current_mode='valid', baseline_mode='valid', timeout=5):
        self.root = Path(root)
        self.binaries = {}
        for name, mode in [('baseline', baseline_mode), ('current', current_mode)]:
            source = self.root / name
            (source / 'src').mkdir(parents=True)
            (source / 'target/debug').mkdir(parents=True)
            (source / 'Cargo.toml').write_text("[package]\nname='antislop'\nversion='0.1.0'\n")
            (source / 'src/main.rs').write_text('original' if name == 'baseline' else 'version branch')
            binary = source / 'target/debug/antislop'
            body = f'''#!{sys.executable}
import os, pathlib, subprocess, sys, time
mode={mode!r}; name={name!r}; a=sys.argv[1:]
if mode=='edit-source':pathlib.Path({str(source / 'src/main.rs')!r}).write_text('source changed during measurements')
if mode=='hang':
    pathlib.Path({str(self.root / 'measured-pid')!r}).write_text(str(os.getpid()))
    child=subprocess.Popen([sys.executable,'-c',"import pathlib,time;time.sleep(3);pathlib.Path("+{str(self.root / 'escaped-descendant-effect')!r}.__repr__()+").write_text('escaped')"])
    pathlib.Path({str(self.root / 'measured-descendant-pid')!r}).write_text(str(child.pid))
    time.sleep(10)
if a and a[0]=='--version' and name=='current':
    sys.stdout.write('antislop 0.1.0' + ('\\n' if mode!='bad-version' else ''));sys.exit(0)
if a==['--help']:
    if mode=='unstable-help':
        counter=pathlib.Path('counter');n=int(counter.read_text()) if counter.exists() else 0;counter.write_text(str(n+1));print('pair',n//2);sys.exit(0)
    sys.stdout.write('baseline help\\n' if mode!='changed-help' else 'changed help\\n')
    if mode=='help-stderr':sys.stderr.write('unexpected stderr\\n')
    sys.exit(0 if mode!='bad-help-exit' else 1)
sys.stderr.write('unknown command\\n' if mode!='changed-error' else 'different error\\n')
if mode=='unknown-stdout':sys.stdout.write('unexpected stdout\\n')
sys.exit(1 if mode!='bad-error-exit' else 0)
'''
            binary.write_text(body)
            binary.chmod(0o700)
            self.binaries[name] = binary
        self.manifest = self.root / 'manifest.json'
        self.manifest.write_text(json.dumps({'source_hashes': benchmark.fingerprint(self.root / 'baseline'), 'provenance': 'independent dummy fixture'}))
        self.payload = benchmark.plan(self.binaries['baseline'], self.binaries['current'], self.manifest, timeout)

    def argv(self, optimized=False):
        code = benchmark.BENCHMARK_CODE.replace('__PLAN__', repr(self.payload))
        return [sys.executable, '-I', *(['-O'] if optimized else []), '-c', code]

    def run(self, optimized=False):
        return subprocess.run(self.argv(optimized), capture_output=True, timeout=90)


class IndependentBenchmark(unittest.TestCase):
    def test_plan_rejects_manifest_source_scope_and_binary_replacement_before_effects(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            self.assertEqual(fixture.payload['pairs'], 30)
            self.assertEqual(len(fixture.payload['workloads']), 5)
            fixture.manifest.write_text(json.dumps({'source_hashes': {'src/main.rs': 'stale'}}))
            with self.assertRaisesRegex(ValueError, 'manifest'):
                benchmark.plan(fixture.binaries['baseline'], fixture.binaries['current'], fixture.manifest)
            fixture.manifest.write_text(json.dumps(benchmark.fingerprint(Path(root) / 'baseline')))
            (Path(root) / 'current/Cargo.toml').write_text("[package]\nname='changed'\nversion='0.1.0'\n")
            with self.assertRaisesRegex(ValueError, 'only src/main.rs'):
                benchmark.plan(fixture.binaries['baseline'], fixture.binaries['current'], fixture.manifest)
        for changed in ['binary', 'source']:
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as root:
                fixture = Fixture(root)
                target = fixture.binaries['current'] if changed == 'binary' else Path(root) / 'current/src/main.rs'
                target.write_text(target.read_text() + '\nchanged after scope pinning\n')
                output = fixture.run()
                self.assertNotEqual(output.returncode, 0)
                self.assertIn(b'changed', output.stderr)
                self.assertFalse((Path(root) / 'measured-pid').exists())

    def test_exact_stream_status_oracles_survive_optimized_python(self):
        for mode in ['changed-help', 'help-stderr', 'bad-help-exit', 'changed-error', 'unknown-stdout', 'bad-error-exit', 'bad-version']:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as root:
                fixture = Fixture(root, current_mode=mode)
                # Fail early for argv cases, and exercise one successful pair
                # before the candidate-only version oracle when appropriate.
                fixture.payload['pairs'] = 2
                output = fixture.run(optimized=True)
                self.assertNotEqual(output.returncode, 0, output.stdout.decode())
                report = json.loads(output.stdout)
                self.assertTrue(report['failure'])

    def test_real_child_cpu_rss_raw_samples_and_balanced_randomization_are_honest(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            before = benchmark.fingerprint(Path(root))
            output = fixture.run()
            self.assertEqual(output.returncode, 0, output.stderr.decode() + output.stdout.decode()[-1500:])
            report = json.loads(output.stdout)
            self.assertEqual(len(report['version_current_only']), 5)
            self.assertTrue(all(row['comparable_baseline'] is False for row in report['version_current_only']))
            self.assertEqual(len(report['measurements']), 5)
            self.assertTrue(any('p95/p99' in limitation for limitation in report['limitations']))
            self.assertTrue(any('polling' in limitation for limitation in report['limitations']))
            for workload in report['measurements']:
                columns = workload['sample_columns']
                samples = [dict(zip(columns, row, strict=True)) for row in workload['samples']]
                self.assertEqual(len(samples), 30)
                self.assertEqual([row['first_binary'] for row in samples].count('baseline'), 15)
                self.assertEqual([row['first_binary'] for row in samples].count('current'), 15)
                self.assertGreater(len(set(row['first_binary'] for row in samples[:15])), 1)
                for name in ['baseline', 'current']:
                    for metric in ['wall_ns', 'cpu_user_seconds', 'cpu_system_seconds', 'peak_rss_bytes']:
                        values = [row[name + '_' + metric] for row in samples]
                        summary = workload[name][metric]
                        self.assertEqual(summary['n'], 30)
                        self.assertTrue(math.isclose(summary['p50'], statistics.median(values), rel_tol=1e-12, abs_tol=1e-12))
                        self.assertEqual(summary['max'], max(values))
                        self.assertLessEqual(summary['p50'], summary['p95'])
                        self.assertLessEqual(summary['p95'], summary['p99'])
                        self.assertLessEqual(summary['p99'], summary['max'])
                        self.assertTrue(all(value >= 0 for value in values))
                    self.assertTrue(all(row[name + '_peak_rss_bytes'] > 0 for row in samples))
                self.assertEqual(workload['paired_wall_ratio']['n'], 30)
            self.assertTrue(all(len(row['samples']) == 30 for row in report['version_current_only']))
            self.assertEqual([row['argv_bytes'] for row in report['unknown_argv_scaling']], [32, 256, 2048, 8192])
            self.assertEqual(benchmark.fingerprint(Path(root)), before)

    def test_rss_units_and_cpu_are_per_specific_waited_child(self):
        for platform, expected in [('darwin', 123), ('linux', 123 * 1024)]:
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as root:
                fixture = Fixture(root)
                fixture.payload['pairs'] = 2
                fixture.payload['workloads'] = fixture.payload['workloads'][:1]
                code = fixture.argv()[-1]
                # Keep real fork/wait/reaping, replace only its accounting record.
                accounting = f'''\nimport types
_actual_wait4=os.wait4
def _accounting_wait4(pid,flags):
    waited,status,usage=_actual_wait4(pid,flags)
    return waited,status,types.SimpleNamespace(ru_maxrss=123,ru_utime=.25,ru_stime=.5)
os.wait4=_accounting_wait4
sys.platform={platform!r}
'''
                code = code.replace('\nPLAN = ', accounting + '\nPLAN = ', 1)
                output = subprocess.run([sys.executable, '-I', '-c', code], capture_output=True, timeout=10)
                self.assertEqual(output.returncode, 0, output.stdout.decode() + "\n" + output.stderr.decode())
                report = json.loads(output.stdout)
                workload = report['measurements'][0]
                rows = [dict(zip(workload['sample_columns'], row, strict=True)) for row in workload['samples']]
                for row in rows:
                    for name in ['baseline', 'current']:
                        self.assertEqual(row[name + '_peak_rss_bytes'], expected)
                        self.assertEqual(row[name + '_cpu_user_seconds'], .25)
                        self.assertEqual(row[name + '_cpu_system_seconds'], .5)

    def test_between_sample_output_drift_and_postmeasurement_source_edits_fail(self):
        for mode in ['unstable-help', 'edit-source']:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as root:
                fixture = Fixture(root, current_mode=mode, baseline_mode=mode if mode == 'unstable-help' else 'valid')
                fixture.payload['pairs'] = 4
                output = fixture.run()
                self.assertNotEqual(output.returncode, 0)
                report = json.loads(output.stdout)
                self.assertIn('changed', report['failure'])

    def test_generator_emits_literal_code_and_pins_manifest_binary_and_source_hashes(self):
        with tempfile.TemporaryDirectory(prefix='literal $(touch NEVER_EXECUTE) ') as root:
            fixture = Fixture(root)
            destination = Path(root) / 'argv.json'
            generated = subprocess.run([sys.executable, str(MODULE), '--baseline', str(fixture.binaries['baseline']), '--current', str(fixture.binaries['current']), '--baseline-manifest', str(fixture.manifest), '--output', str(destination), '--python', sys.executable, '--timeout', str(fixture.payload['timeout_seconds'])], capture_output=True, timeout=5)
            self.assertEqual(generated.returncode, 0, generated.stderr.decode())
            argv = json.loads(destination.read_text())
            self.assertEqual(argv[:3], [str(Path(sys.executable).resolve()), '-I', '-c'])
            self.assertEqual(len(argv), 4)
            metadata = json.loads(generated.stdout)
            self.assertEqual(metadata['code_sha256'], hashlib.sha256(argv[3].encode()).hexdigest())
            assignment = next(node for node in ast.parse(argv[3]).body if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == 'PLAN' for target in node.targets))
            self.assertEqual(ast.literal_eval(assignment.value), fixture.payload)
            self.assertEqual(metadata['baseline_manifest_sha256'], hashlib.sha256(fixture.manifest.read_bytes()).hexdigest())
            self.assertFalse((Path.cwd() / 'NEVER_EXECUTE').exists())
            self.assertFalse((Path(root) / 'NEVER_EXECUTE').exists())


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--emit-hanging-fixture':
        fixture = Fixture(sys.argv[2], baseline_mode='hang', timeout=1)
        print(json.dumps(fixture.argv()))
    else:
        unittest.main()
