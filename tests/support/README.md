# Test resources

Run Rust tests with `./cargo.sh test ...`. Cargo invokes `run_test.py` separately
for each executable, before libtest starts threads. Direct execution must use
`python3 -B tests/support/run_test.py PATH_TO_TEST_BINARY [ARGS...]` as well.
The runner keeps cwd by default and imposes no access or execution budgets.
The historical comparison helper alone sets ANTISLOP_TEST_EXTERNAL_CWD=1 to
run old executables with cwd in their owned active directory, since their frozen
helpers predate this contract. Only `1` and unset are accepted; invalid/empty
values fail. This compatibility setting is passed to the launcher, then removed
from the child environment.
It supplies environment through `Popen(env=...)`; tests must not mutate global
environment or cwd while other threads exist. Recursive subprocess tests inherit
this environment. Child-specific overrides belong on `Command.env`.

`resources.py::resources` owns a unique mode-0700 active directory under `/tmp`
(canonicalized, no symlink alias). `ANTISLOP_TEST_TEMP_ROOT`, when present, must
name an existing external directory whose canonical encoded path is shorter than
20 bytes. Invalid, empty, source-local (including symlinked), or long roots fail;
there is no fallback. Inherited TMPDIR is deliberately ignored when allocating
this root. The runner sets TMPDIR to its short private `tmp` child so ordinary
`tempfile` users are isolated too. It creates config/state directories and sets
ANTISLOP_CONFIG_DIR and ANTISLOP_STATE_ROOT before test startup. Fixture-only Agy
and Gemini bridge checks are disabled in this child environment, without editing
provider configuration. No turn timeout override is installed.

Rust `common::tempdir` and the library's cfg(test) `test_resources::tempdir`
allocate unique TempDirs under ANTISLOP_TEST_ROOT. This variable is required,
absolute after canonicalization, outside CARGO_MANIFEST_DIR, and shorter than
50 bytes. Each fixture owns its directory; keep the guard alive until all users,
threads, and processes finish. Long project paths are allowed, but daemon state
must have a separate short path. `common::assert_test_socket(state, child_cwd)`
checks the actual resolved length, external location, and absence of daemon socket
fallback. Use the same state spelling for bind and clients. Never create a
checkout-local alias. Tests intentionally covering fallback sockets must own and
unlink exactly their socket, not sweep shared directories.

`ChildGuard::new(child)` must wrap a spawn immediately, before readiness checks.
It kills and reaps a still-running child on unwind or normal return. Daemon
fixtures use common::stop_daemon (Client::stop, which waits for the state lock)
before the child guard reaps the process; stop/join background
workers before dropping their directories. Fields drop in declaration order, so
when a fixture owns both, explicitly stop the child in its Drop before its
TestDir field is released. Python uses `with process(...)` inside
`with resources()` for the same ordering. A failed spawn creates no process to
clean up. Synchronous `Command.output`/`subprocess.run` already wait for children.
A process intentionally detached by a CLI needs DaemonStateGuard around its
allocated state for the entire consumer scope; it is installed before autostart.
A child guard alone does not own arbitrary descendants. Detached daemon children
can be reaped with waitpid when the consumer has their PID; the regression checks
that path as well.

ANTISLOP_TEST_DIAGNOSTICS optionally selects an external durable diagnostic
parent. Default: `$XDG_STATE_HOME/antislop/test-diagnostics`, else
`~/.local/state/antislop/test-diagnostics`. The Python context creates a unique
run directory, then passes that run directory to Rust. Empty or source-local
values fail before creating anything there. Rust copies only regular `*.log`,
`*.stderr`, `*.stdout`, `*.prompts`, `*.rejections`, `*.reminders`, `*.keys`,
`stderr`, and `stdout` files (no symlinks or `.git` trees)
before deleting its own fixture, on success and unwind. The runner stores command,
exit status and streamed output and copies remaining selected logs before removing
its active root. Do not put arbitrary repository contents into diagnostic files.
A diagnostic copy failure retains the owned active directory and reports its path;
cleanup errors fail a normally returning test and are printed during an existing panic, without a second panic. Diagnostic
runs are retained intentionally; their caller owns retention. Never delete
preexisting ignored state or another run based on its name.

Replay cleanup owns active relay connections and shuts them down before joining
workers, including when daemon stop fails. Late connections are closed immediately
after shutdown begins; the original failure still retains fixture diagnostics.

`tests/test_resources.py` and `lean_test_resources` exercise parallel allocation,
bind/connect with long project paths, panic/readiness failure, failed spawn,
reaping, selective diagnostic retention, neighbor preservation and environment
stability. Drop/finally cannot clean after SIGKILL or host failure; the printed
active path identifies such an orphan, but no automatic sweep is authorized.

Manual Python fixtures with explicit `--root` must use a new external diagnostic
parent. They allocate active fixtures through `resources(diagnostic_root=...)`
and reap their Popen children through an ExitStack before cleanup. Only selected
logs and structured diagnostic events are retained. The keyword diagnostic_root,
when supplied, overrides the diagnostic parent environment setting; accepted
values follow the same external-path contract.
`record_options_checks.py` requires ANTISLOP_OPTIONS_EVIDENCE naming a new external
output directory, and uses the current cargo.sh runner even for archived sources.
The archived baseline is compiled in an external temporary source snapshot and
its tests run with external cwd; it never writes state into that source snapshot.

The resource owners do not promise cleanup after a program deliberately removes
its own fixture root or escapes ownership. For in-process Core consumers the
launcher supplies one immutable config/state environment shared by that test
executable; use per-fixture Core state and project paths within it. A consumer
requiring a different global environment must use a separate child executable,
not mutate the parent environment.

The message/recovery/consent sequence tests now use this common contract instead
of ANTISLOP_MESSAGE_SEQUENCE_RUN_DIR, ANTISLOP_TUI_RECOVERY_EVIDENCE, or
ANTISLOP_TASK_STATE. Those former fixture-location variables are no longer read.
Set ANTISLOP_TEST_DIAGNOSTICS to your own durable parent for retained logs.
Generated provider scripts and shims must be placed in an allocated fixture;
common/support installers validate the canonical parent against the source root
before modifying it.

Bootstrap replay restores its renamed listener only while the endpoint is absent
or still belongs to its proxy; a rebound successor endpoint is preserved. Cleanup
reaps the frontend before stopping the daemon, checks the stop result and core.lock,
and joins all relay workers even after an earlier cleanup error. RetainResources
means release could not be confirmed: selected logs are copied, but the active
directory is retained and its exact path is printed for diagnosis.
Run `python3 -B tests/test_replay_cleanup.py` after building the debug binary to
exercise controlled failures after rename and proxy installation, before handoff.

## Render checks

`tests/test_reference_renders.py` tested the retired Python archive codec, not
rendering: exact byte round trips, gzip/base64 validation, length/SHA-256 checks,
duplicate keys and refusal to overwrite output. Commit `d07b7f4a2` removed the
codec together with its historical render archives and adapter documentation.
The orphaned test is removed too; compatibility with that historical envelope
and its publication behavior is no longer checked or provided.

Current rendering remains checked by the ordinary library tests
`lean::tui::capture::fixture_contracts_reach_the_requested_views_and_export_full_buffers`
and `lean::tui::capture::semantic_contracts_reject_corrupted_capture_evidence`.
They render through real `App::draw`/TestBackend, validate fixture registration,
geometry and complete cell buffers, and reject corrupted state, style and cells.
`lean_tui` adds rendering and input regressions. CI runs these through
`./cargo.sh test --locked --all-targets`, using the resource launcher above.
For targeted capture regressions, run:

```sh
./cargo.sh test --lib lean::tui::capture::
```

The ignored `export_catalog` test is an optional current JSON export, not a
historical archive decoder. Ordinary render checks do not write a catalog.
Manual export requires `ANTISLOP_CAPTURE_DIR` naming a new external directory;
keep it outside sources and use the same launcher and durable diagnostics.
