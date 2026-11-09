# Changelog

Each release lists what changed for people using antislop. Draft a section
from the commits since the last tag with `packaging/changelog.sh`.

## Unreleased

- `antislop doctor` checks the configuration, the state directory and that
  every profile's CLI launches; a plan no longer starts while a CLI it needs
  is missing, and the refusal names the profiles that would launch.
- `antislop init` writes a starting `antislop.toml` from the CLIs on `PATH`.
- `--version` prints the build identity.
- Several terminals can follow one conversation: the first to act on it
  steers, the others watch until they take control from the palette.
- Retried requests (send, reject, resume, cancel, approval answers, review
  comments, orchestrator restart) act once and return their first result.
- Agy command, MCP, edit and outside-workspace approval dialogs are recognized
  and answered through the runtime.
- Landed and repaired task commits keep the subject their worker wrote.
- Release archives for macOS and Linux, and a Homebrew formula.

## 0.1.0

First version: an orchestrator, planner and per-task worker/reviewer pairs
running as interactive CLIs (Codex, Claude Code, Agy, Gemini CLI, OpenCode)
in daemon PTYs, with requirements ledgers, plan review, isolated task
worktrees, integration that preserves user edits, a final audit and a TUI.
