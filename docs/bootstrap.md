# Conditional executable bootstrap

`antislop bootstrap` publishes a reviewed candidate under the **verified executable
path used by the existing frontend**. It does not change code already in memory
and does not activate or roll back a daemon. The operator supplies that path;
working directory and PATH are not substitutes for identifying it.

This is conditional atomic replacement, not compare-and-swap. Before each
`publish`, `resume`, `cancel`, or `restore`, identify and stop **all** other writers
of the target and its directory: package managers, IDE updates, installation
scripts, commands in other terminals, and any other updater. Prevent new writes
until the final identity check, rename, directory sync and durable journal
settlement finish. `--exclusions TEXT` records concrete exclusions and prevention
for this operation. A newly generated stdin challenge records the operator's
current confirmation. An earlier confirmation cannot be reused after interruption.
The stable `.antislop-bootstrap.lock` serializes cooperating publishers only;
it cannot exclude tools that disregard the operator's window. If the window
cannot be established, do not confirm. No production confirmation is needed for
an isolated fixture controlled by its test operator.

## Publication and delivery order

Use an independently reviewed candidate and a new absolute durable journal
directory **outside disposable checkouts**. Its parent must already exist. The
legacy target must be a regular executable, not a symlink; resolve and verify the
actual frontend path first. The operator must be able to create entries in its
directory. Setuid/setgid and owner/group changes are rejected. Do not modify
candidate or final target metadata during handoff: runtime build identity includes
canonical path, size and mtime, while the journal additionally records SHA-256,
device/inode, mode and owner/group. An archive path has its own build identity.

```sh
CANDIDATE bootstrap prepare --target LEGACY_EXECUTABLE --candidate CANDIDATE --journal DURABLE_NEW_DIR
CANDIDATE bootstrap status --journal DURABLE_NEW_DIR
CANDIDATE bootstrap publish --journal DURABLE_NEW_DIR --exclusions 'CONCRETE stopped updaters and prevention of new writes'
# Establish the window, then answer the displayed fresh CONFIRM challenge on stdin.
antislop activate --binary LEGACY_EXECUTABLE --conversation CONVERSATION --task TASK --config CONFIG --state STATE
antislop activate --status --request-id REQUEST_ID --state STATE
```

`CANDIDATE` is the command path of the reviewed binary containing this CLI.
`LEGACY_EXECUTABLE` is the final publication path, not the archive or build path.
`activate` prints its request ID; recording `applied` must be paired with the live
daemon's PID, executable path/hash and build matching that final path. Pending,
CoreReady, and an Identity response alone do not prove completion. Production
publication/activation belongs to the separate delivery task after independent
review. Implementers use fixture paths only.

## Recovery

```sh
CANDIDATE bootstrap status --journal DURABLE_NEW_DIR
CANDIDATE bootstrap resume --journal DURABLE_NEW_DIR --exclusions 'NEW concrete exclusions and prevention'
CANDIDATE bootstrap cancel --journal DURABLE_NEW_DIR --exclusions 'NEW concrete exclusions and prevention'
CANDIDATE bootstrap restore --journal DURABLE_NEW_DIR --exclusions 'NEW concrete exclusions and prevention'
```

For **each** command re-establish the window and answer its new challenge. A new
process cannot consume a persisted confirmation. `resume` finishes publication
or an in-progress restore by checking the actual target against the recorded
original, published inode, or restore inode. `cancel` durably settles the observed
file and prevents further publication; it retains every file and does not undo a
completed rename. A cancelled publication can still be restored. An in-progress
restore must be settled by `restore`/`resume`, not cancelled. `restore` copies the
retained original into another inode and atomically publishes that file; it
preserves bytes, mode, owner/group and mtime. It never overwrites the running inode.
Restored inode and build identity can differ from the historical original.

Preparation stores a synced original and candidate copy in the private journal
directory, then a synced stage in the target directory (same filesystem), plus
immutable numbered journal records. It records original identity and metadata
before any target replacement. Files and journal are retained on all refusals.
No automatic rollback is attempted:

- Copy/validation failure: target is unchanged; partial copies and any available
  journal remain. If preparation never produced a journal, preserve that directory
  and start a new preparation in another directory; there was no target mutation.
- Rename failure: durable `publishing` intent and all surviving files remain;
  recovery checks the actual target, never assumes the rename succeeded.
- Directory sync or final journal failure: publication is **unsettled**, even when
  the target is already B. Preserve evidence; a renewed window and `resume` sync
  and settle it. Immutable preceding records survive a failed later record.
- Restore file sync failure: retain the complete stage and leave the target
  unchanged. After reopening under a fresh confirmed window, verify and sync
  the stage again before recording restoring intent or renaming it. Matching
  hash/metadata and directory sync do not establish file data durability.
- Crash before/after rename: target identity distinguishes original from staged
  inode. A fresh window is mandatory even if a prior process held the lock.
- Target or target directory mismatch, symlink, changed stage, missing/stale
  confirmation: refuse destructive action and retain all versions. Later user
  changes are never unconditionally overwritten. A damaged/incomplete restore
  stage is retained and refused; a new preparation using the retained original
  as candidate is possible only after identifying the current target again.

Keep journal, original/candidate archives, stages and the stable lock; the CLI has
no destructive cleanup command. The journal does not promise preservation of
extended ACLs/xattrs beyond the executable's recorded POSIX metadata; deployments
requiring those need assessment before using this procedure. File rollback alone
never establishes which daemon is live. A writer violating the operator window
can still race the final check and rename; this CLI makes no stronger guarantee.

## Isolated replay

```sh
python3 tests/bootstrap_replay.py --legacy ARCHIVED_ACTUAL_A --candidate REVIEWED_B --root NEW_DURABLE_FIXTURE_DIR
```

Use `--loss-phase switching` to close Events immediately after the durable
Switching record; the default `start` closes it after the successor rebinds the
listener. The observed journal states are recorded: a fast run may already be
Applied at the latter boundary. This is not a claim that every start injection
occurred before confirm.

Legacy A has only one reconnect/replay attempt. Loss in the exec interval can
therefore end that frontend with raw `the daemon disconnected` even while B
successfully confirms Applied and retains all durable work. Publication changes
future launches, not that code in memory. After checking Applied and live B, the
supported continuation is the final executable with the existing conversation:

```sh
LEGACY_EXECUTABLE chat --conversation CONVERSATION --config CONFIG --state STATE
```

This reconnects to the same B daemon when its context matches; it is not a manual
daemon restart. Keep the old failure and the continuation command as evidence.
Do not infer delivery readiness from the frontend exit or from historical Applied.

The harness retains commands, operator preconditions, challenge, request/status,
PID/path/hash/build, frontend logs and journals. A fixture socket relay closes a
real A Events response at the selected boundary; no production hook, service or
watcher is installed. A later isolated daemon loss checks that the old frontend
starts B under its original path, and explicit stop checks that it stays stopped.

`python3 tests/bootstrap_cli.py --binary REVIEWED_B --legacy ARCHIVED_ACTUAL_A
--root NEW_DURABLE_FIXTURE_DIR` additionally exercises real CLI refusals for missing,
stale and interrupted windows, renewed resume, cancel, restore, and target changes
before/after publication. Unit tests inject filesystem errors in test builds only;
the released CLI has no fault-injection environment or control interface.
