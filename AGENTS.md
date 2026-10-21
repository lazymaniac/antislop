# AGENTS.md — Development Guidelines for antislop

This document specifies the rules, design constraints, and engineering practices that all AI coding agents must follow when working in this repository.

---

## 1. Project Overview & Crate Layout

- **Binary Crate**: `antislop` (`src/main.rs`) — Terminal UI and daemon entrypoint.
- **Library Crate**: `antislop` (`src/lib.rs`) — Core scheduling, storage, agent drivers, and TUI engine.
- **Subsystems**:
  - `src/lean/`: The lean core runtime, SQLite event store, task DAG manager, daemon, and Git worktree coordinator.
  - `src/lean/tui/`: TUI implemented with `ratatui` and `crossterm`.
  - `src/lean/agent.rs` & `src/lean/cli.rs`: Process wrappers and event streaming for agent CLIs (`codex`, `agy`, `claude`, `gemini`).

---

## 2. Toolchain & Rust Standards

- **Rust Edition & Channel**: Edition `2024`, Rust `1.98.1` (`rust-toolchain.toml`).
- **Formatting**: Always format code using standard Rust formatting:
  ```sh
  cargo fmt --check
  ```
- **Linting & Warnings**: Zero tolerance for compiler or clippy errors:
  ```sh
  cargo clippy --all-targets -- -D warnings
  ```
- **Verification**: Run `cargo check` and targeted tests before concluding changes:
  ```sh
  cargo test --test lean_core
  cargo test --test lean_tui
  ```

---

## 3. Core Architectural Principles (Lean Core)

1. **Agents Work Directly on Files**:
   - Never copy entire repository trees or full file contents into SQLite database tables, event payloads, or prompt bodies.
   - Agents explore directories independently using their CLI tools.
   - Writers work in isolated Git worktrees; verifiers inspect diffs or read worktrees in read-only mode.

2. **Compact, Scalable State**:
   - Database operations in `src/lean/store.rs` must update small, normalized rows one at a time with an append-only event log.
   - Avoid whole-document hashing or state dumps that scale with the repository size.

3. **No Unactionable Blockers**:
   - When a CLI execution or background step fails, report the raw failure reason directly so the user can retry or adjust.
   - On daemon restart or interruption, mark in-flight turns as interrupted cleanly.

---

## 4. Terminal UI (TUI) & Text Editing Rules

When modifying or adding UI components in `src/lean/tui/`:

1. **Unicode & Grapheme Integrity**:
   - All text slicing, cursor movement, line wrapping, and deletions must operate on **Unicode extended grapheme cluster boundaries** (`unicode_segmentation`), never on raw bytes or character counts.
   - Support zero-width joiners (ZWJ) and combining marks without splitting glyphs or corrupting UTF-8.

2. **Draft & State Preservation**:
   - Network or daemon errors must never discard user text. Unsent drafts must be recovered or restored.
   - Retain drafts independently per conversation during conversation switching (`Ctrl+O`).
   - Modal dialogs (Help, Comments, Confirmation Cards) must cleanly isolate key events and preserve background drafts and search queries.

3. **Terminal Dimensions & Canvas Bounds**:
   - Guard against terminal dimensions smaller than 12 columns x 5 rows.
   - Keep canvas memory allocations bounded by viewport area (`O(viewport_area)`). Never allocate canvas buffers proportional to graph task count.
   - Honor `NO_COLOR=1` (disable ANSI colors, keep dim/reversed) and `ANTISLOP_ASCII=1` (render borders in plain ASCII).

---

## 5. Working Tree & Process Safety

- **Working Directory**: The process working directory is the repository root supplied by the user. Do not hardcode absolute system paths.
- **Runtime State**: All runtime state, database files, and socket locks belong in `.antislop/` (ignored in `.gitignore`). Do not write artifacts or logs into the source tree. The one exception is the user's list of trusted directories (`trusted.json` in `~/.config/antislop/`, see `src/lean/trust.rs`): it belongs to the user, not to a project, and nothing may be created in a directory before it is trusted.
- **Git Worktree Isolation**: Task workers must only modify files inside their allocated task worktrees. Never mutate the user's primary working tree outside approved merges.
- **External CLI Processes**: When spawning agent CLIs, always enforce timeout handling, stdout/stderr streaming, and explicit cancellation channels.
