"""Owners for short active resources and separate durable diagnostic directories."""
import contextlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import traceback

class RetainResources(RuntimeError):
    """Cleanup cannot confirm release; keep the owned active directory."""


SOURCE = Path(__file__).resolve().parents[2]


def external(path, *, strict=True):
    if not os.fspath(path):
        raise ValueError('resource path must not be empty')
    path = Path(path).resolve(strict=strict)
    if path == SOURCE or SOURCE in path.parents:
        raise ValueError(f'test resources must be outside sources: {path}')
    return path


def preserve_logs(source, target):
    """Select logs only, without following links or copying repositories."""
    for folder, dirs, files in os.walk(source, followlinks=False):
        dirs[:] = [d for d in dirs if d != '.git' and not Path(folder, d).is_symlink()]
        for name in files:
            path = Path(folder, name)
            if not path.is_symlink() and (path.suffix in ('.log', '.stderr', '.stdout', '.prompts', '.rejections', '.reminders', '.keys') or name in ('stderr', 'stdout')):
                dest = target / path.relative_to(source)
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, dest)


@contextlib.contextmanager
def resources(*, diagnostic_root=None):
    # Do not use TMPDIR: it may be a long worktree or a source directory.
    root = external(os.environ.get('ANTISLOP_TEST_TEMP_ROOT', '/tmp'))
    if len(os.fsencode(root)) >= 20:
        raise ValueError(f'ANTISLOP_TEST_TEMP_ROOT must be shorter than 20 bytes: {root}')
    diagnostic_root = diagnostic_root if diagnostic_root is not None else os.environ.get('ANTISLOP_TEST_DIAGNOSTICS',
        Path(os.environ.get('XDG_STATE_HOME') or Path.home() / '.local/state') / 'antislop/test-diagnostics')
    diagnostic_root = external(diagnostic_root, strict=False)
    diagnostic_root.mkdir(parents=True, exist_ok=True)
    diagnostic_root = external(diagnostic_root)
    diagnostic = Path(tempfile.mkdtemp(prefix='run-', dir=diagnostic_root))
    active = Path(tempfile.mkdtemp(prefix='as-', dir=root))
    try:
        (diagnostic / 'resources.log').write_text(f'active={active}\n')
        env = dict(os.environ)
        for name, subdir in [('TMPDIR', 'tmp'), ('ANTISLOP_STATE_ROOT', 'state'),
                             ('ANTISLOP_CONFIG_DIR', 'config')]:
            path = active / subdir
            path.mkdir(mode=0o700)
            env[name] = str(path)
        env.update(ANTISLOP_TEST_ROOT=str(active), ANTISLOP_TEST_DIAGNOSTICS=str(diagnostic),
                   ANTISLOP_SKIP_AGY_BRIDGE_CHECK='1', ANTISLOP_SKIP_GEMINI_SETTINGS_CHECK='1',
                   PYTHONDONTWRITEBYTECODE='1')
        yield active, diagnostic, env
    except BaseException:
        (diagnostic / 'exception.log').write_text(traceback.format_exc())
        raise
    finally:
        retain = isinstance(sys.exc_info()[1], RetainResources) or (active / 'retain-resources.log').exists()
        try:
            preserve_logs(active, diagnostic / 'remaining')
        except BaseException:
            print(f'diagnostic copy failed; active resources retained at {active}', flush=True)
            raise
        else:
            if retain:
                print(f'unconfirmed resource release; active resources retained at {active}', flush=True)
            else:
                try:
                    shutil.rmtree(active)
                except BaseException:
                    print(f'active resource cleanup failed: {active}', flush=True)
                    raise


@contextlib.contextmanager
def process(argv, **kwargs):
    """Install ownership before readiness checks; always reap our child."""
    child = subprocess.Popen(argv, **kwargs)
    try:
        yield child
    finally:
        if child.poll() is None:
            child.kill()
        child.wait()
        for stream in (child.stdin, child.stdout, child.stderr):
            if stream is not None and not stream.closed:
                stream.close()
