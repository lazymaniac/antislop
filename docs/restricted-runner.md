# Restricted check runner: capability spike

`design.md` §13 requires an actual enforcement boundary and explicitly leaves
the macOS/Linux mechanism to a spike. The existing worktree and process guardian
do not supply that boundary. Autonomous project checks remain refused by
`Policy::authorize_project_check`; this experiment does not change policy,
configuration, approval records or check evidence.

Run the owned, provider-free experiment explicitly:

```sh
python3 scripts/probe_restricted_runner.py
```

It executes fixed disposable Python programs in a private temporary fixture,
using macOS `sandbox-exec` and CommandLineTools Python. It never executes a
repository command or loads user/provider configuration. Canonical directory
paths are passed as SBPL parameters, including spaces, quotes, brackets and
backslashes, rather than interpolated into policy expressions. The child receives
an explicitly constructed environment, a private home/temp directory, null stdin
and no inherited authentication variables. Standard error is discarded; the
report contains only the profile hash, platform, booleans and errno/status values.
Temporary fixtures are removed. The tool does not install or start a VM/container.

The sampled policy denies operations by default, allows selected-source reads,
scratch writes and narrow system/tool roots, and grants no network access. The
experiment tests protected reads/writes, writes to selected sources, symbolic and
hard links, TCP loopback, Unix sockets, child restrictions and a child's `setsid`.
Unsandboxed controls establish working owned listeners and accessible targets;
source-integrity checks run afterward. Missing observations, missing targets,
connection refusal and unrelated errors cannot count as successful restrictions.
The fixed child that calls `setsid` exits immediately; this is not a hostile
descendant-cleanup test. No arbitrary command is admitted through this probe.

On Darwin 25.6.0 on 2026-10-16, selected reads and scratch writes succeeded;
all eight sampled forbidden file/network operations and the child's outside
write returned EPERM. Both source sentinels stayed unchanged. A child could call
`setsid`, so the result was **rejected**, exit 2, `autonomous_ready: false`.
The existing process-group guardian cannot contain deliberately escaped sessions.
This policy therefore cannot be advertised as the autonomous runner. The initial
separate exploratory attempt also found that `/usr/bin/python3` is an Xcode
dispatcher needing additional host access; the committed probe uses the explicit
CommandLineTools interpreter instead of widening its policy for the dispatcher.

Exit 2 also covers unavailable/inconclusive execution. Exit 0 means only the
sampled restrictions passed; `autonomous_ready` remains false. Resource limits,
arbitrary hostile-descendant containment, system read-root review, toolchain
compatibility, Linux enforcement and complete runtime authorization/evidence
binding remain untested. This report is diagnostic data, not a capability pin,
project-check receipt or completed security acceptance.

The next backend evaluation must prove descendant containment, deny host secrets
and sockets, bind exact read-only check inputs with separate writable scratch,
and fail closed on unavailable enforcement. A Linux namespace/container backend
needs its own executable positive/adversarial acceptance on Linux. A macOS VM
may offer that boundary but availability and compatibility must be measured;
an installed container CLI alone establishes neither. No stronger mode is enabled
from these observations.

Primary implementation references reviewed for the spike:

- [Anthropic sandbox runtime](https://github.com/anthropics/sandbox-runtime)
  and its [macOS policy construction](https://github.com/anthropics/sandbox-runtime/blob/main/src/sandbox/macos-sandbox-utils.ts).
- [Bubblewrap documentation](https://github.com/containers/bubblewrap/blob/main/README.md).

These are implementation references, not proof that Architect enforces their
guarantees. No third-party sandbox code was vendored and no dependency was added.

## Container alternative: executable spike

An already running rootless Podman machine and a local Rust/Python image were
available on this Mac. The second experiment uses them explicitly:

```sh
python3 scripts/probe_container_runner.py --image sha256:LOCAL_IMAGE_ID
```

The argument must be a complete 64-hex image ID. Mutable tags are refused;
`--pull=never` prevents an implicit download. This tool creates only uniquely
named, labelled disposable containers and removes only containers carrying its
own per-invocation ownership token. It neither starts nor stops the machine and
does not alter other containers. Inspecting the effective configuration precedes
starting each fixture. Added host bind mounts, writable selected-source mounts,
extra image volumes, host network/PID/IPC settings, elevated privileges, extra
capabilities, host devices, published ports and missing configured limits refuse
start. Image/proxy environment inheritance is disabled explicitly.

The main fixture mounts only its generated input directory read-only, with
executable scratch in private tmpfs. Kernel observations verify no effective
capabilities, `NoNewPrivs=1`, seccomp enabled, loopback-only interfaces and actual
cgroup CPU/memory/PID limits. Independent commands compile and run a passing
Rust test and an intentionally failing Rust test. Forbidden writes, a hard-link
attempt, an unmounted host sentinel, a symbolic link to that sentinel, a host
engine socket and an unroutable network attempt have separate expected outcomes.
Missing host paths count only alongside exact inspected binds and the positive
selected-read control; an arbitrary connection error cannot prove isolation.

A second generated fixture adds only its own host-writable heartbeat directory.
Its child changes session and process group; the host observes a growing
heartbeat before killing the container, then a stopped container/exit 137 and
unchanged heartbeat. This specifically tests the escape that defeated the
Seatbelt/process-group combination. It is a sampled stop observation, not proof
of daemon-crash recovery or universal hostile-process containment.

On Darwin 25.6.0 on 2026-10-16, local image
`sha256:c1999f81da8e1f787006d58cbaece8ac4837d7ae04ad16784d829470f4bebc8d`
passed these sampled restrictions and stop observations; all owned containers
were removed. Rust compilation and the positive test exited 0; the negative test
exited 101. Actual limits read 256 MiB memory, 32 processes and one CPU quota.
The host sentinels and selected source stayed unchanged. No user repository,
credential directory, daemon socket or authentication store was mounted.

The JSON report still declares `autonomous_ready: false`. Runtime admission,
policy/image/environment pinning, exact input/evidence receipts, cancellation and
daemon-death recovery must be implemented before this can authorize project
checks. Native Linux-host acceptance, arbitrary toolchains, hostile resource
exhaustion, engine-configuration trust and source-secret selection remain open.
The existing autonomous project-check guard remains in force. A passed spike
does not change old approvals or turn a diagnostics report into check evidence.

The probe's 256 MiB/32-process/one-CPU settings are disposable experiment
controls, not proposed product defaults. Section 14 of the original design
forbids application-imposed or hidden MVP task budgets, agent/concurrency/time/
cost/delegation/correction limits. A production runner must preserve that
contract rather than copy this sampled quota configuration. Actual OS, engine
and provider constraints must be reported as such. Transport frame bounds and
ownership-observation deadlines must not be presented as task-completion budgets.

Command semantics were checked against the primary
[Podman run documentation](https://docs.podman.io/en/latest/markdown/podman-run.1.html).

## Rust guardian: owned container lifetime

`process::command_with_container` now registers an already-created container with
the separate process guardian. `ContainerResource` requires an absolute engine
path, a complete container ID, an immutable image ID and a UUID invocation token
matching `io.antislop.check-invocation`. This is a lifecycle primitive, not a
permission grant or a declaration that the container enforces a sandbox profile.
It is not yet selected by project checks or configuration.

After local command exit, cancellation or parent SIGKILL, the guardian stops and
reaps its local process group, then removes the exact owned container before
releasing inherited workspace leases or reproducing the command's exit/signal.
INT, TERM, HUP and QUIT request this same cleanup instead of killing the guardian
prematurely. They are ignored only across its exec startup window, then handled
before the external command starts; caught dispositions reset on child exec.
An engine-side projection returns only ID, image and the one ownership label;
full inspect environment/mount/label data is not collected by this path. Control
output is bounded to 4 KiB, drained completely and never printed; stderr is
discarded. Control commands use their own supervised groups and a ten-second
observation deadline. A timeout is an unconfirmed cleanup, never proof of absence.

`container exists` must report absence explicitly. Engine failure or an ownership
ID/image/token mismatch leaves the guardian alive with its workspace leases,
retrying cleanup. A single bounded notice explains this pending state; raw engine
errors are not logged. Recovery of the engine/ownership observation or confirmed
external removal allows it to finish. No filter, prefix, latest/all selector,
image removal, volume removal or operation on dependent containers is used.

Ten ordinary executable fixtures cover malformed selectors, direct invocation
without a private lifeline, leader exit, cancellation, failed local launch,
signal preservation, unavailable engine/foreign bindings and parent SIGKILL with
a deliberately delayed removal, malformed/oversized control output and guardian
termination signals. They verify that leases remain held and a
supervisor remains live until cleanup is actually confirmed. The opt-in real
Podman test additionally exercises exit 7, cancellation, guardian TERM and parent SIGKILL with
session-escaping children; its generated containers have no host project mount:

```sh
ANTISLOP_TEST_PODMAN=/absolute/path/to/podman \
ANTISLOP_TEST_CONTAINER_IMAGE=sha256:LOCAL_IMAGE_ID \
scripts/cargo.sh test --locked --test container_supervision \
  real_container_cleanup_covers_exit_cancel_and_parent_sigkill_with_session_escape \
  -- --ignored --nocapture
```

The remaining runner integration must use the durable ownership fences below,
capture immutable selected inputs, validate effective restrictions before start,
pin engine/environment/policy/image capabilities, retain actual task termination
and authenticate check evidence. It must also handle ambiguous pre-start engine
operations and guardian loss/restart. This lifecycle change alone neither accepts
project results nor enables autonomous checks. The existing supervised/native
process paths keep their previous behavior unless a container is registered.

Primary exit/removal references:
[container exists](https://docs.podman.io/en/latest/markdown/podman-container-exists.1.html),
[container rm](https://docs.podman.io/en/latest/markdown/podman-rm.1.html).

## Durable container custody

`runtime::check_resources` records each container invocation in the canonical run
and the existing SQLite state/event transaction. An unused ledger is omitted from
serialization, preserving original native prepared-input hashes. Loading and
saving validate ownership, full IDs, unique bindings and legal transition history.

Reservation binds one persisted task-check intent, exact plan/argv/source and
artifact identity, configuration history and its scoped local approval. A create
fence is issued only after the `create_unknown` checkpoint commits; a start fence
requires a persisted full container ID and a separate `start_unknown` commit.
They cannot be cloned or deserialized and establish checkpoint ordering only.
They do not grant permission or establish the effective sandbox. A failed
transaction changes neither the canonical run nor the caller's in-memory copy.
Creating or starting again through an uncertain boundary is refused.

Cleanup may proceed after pause, changed sources or changed task policy, but its
completion requires a consumed proof returned by the actual engine removal path.
A model/client boolean cannot construct that proof. Cleanup creates no check
success or task evidence. On restart, unresolved custody blocks reconciliation,
preserves cancellation/pause and does not issue engine operations. Source-only
reconciliation cannot settle a potentially live external resource.

Seven executable tests cover reopening at both uncertain boundaries, real SQLite
transaction failures, stale intent/approval/policy, pause/cancellation, engine
absence with failed proof persistence and malformed/foreign/rebound histories.
For a lost create response, a read-only observation selects the exact reserved
name and checks returned name, complete ID, image and invocation label before
issuing a non-deserializable creation observation. The journal consumes it to
record the recovered ID, even after interruption. A missing name yields no proof
and does not settle custody: a submitted create could still complete later.
Unavailable/malformed/oversized responses or foreign bindings cannot recover an
ID. This observation never creates, starts or removes a container; a stopped run
still cannot start one afterward.

The opt-in real Podman experiment additionally exercised valid and foreign-token
name observations before all four lifecycle scenarios. It passed in 6.50 seconds
and removed every owned container (`/tmp/architect-custody-name-owned-podman.log`).
The inspected fields/selection follow the primary
[container inspect documentation](https://docs.podman.io/en/latest/markdown/podman-container-inspect.1.html).
The ordinary observation test also injected a real SQLite failure and proved no
state change, no mutation calls and no fabricated task/check success.

The journal cannot infer absence or authorize a replacement. A production
check controller, recovery admission/UI, configuration/capability pins, immutable input capture,
effective restriction validation and authenticated outcome evidence remain open.
This ledger is not yet selected by project checks and enables no autonomous work.
