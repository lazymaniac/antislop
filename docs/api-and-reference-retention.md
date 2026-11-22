# Retained API and design references

Decision: retain these items in place. Inspection of the resulting source found
no production or test callers before the availability regressions below were
added. That is evidence of no local use, not proof of global redundancy.
`lib.rs` exports `lean`, `lean/mod.rs` exports `store` and `tui`, and
`tui/mod.rs` exports `Details` and `Theme`. `Store` and `MailTarget` are public.
`publish = false` does not rule out external path or Git dependencies. No external
consumer inventory is available. These are unconfirmed redundancy suspicions,
not claims of demonstrated external demand or newly designated extension points.

## Individual API decisions

| Item | Source and reason to preserve it |
|---|---|
| `Store::pending_mail_targets` | `store/comment_mail_state.rs`: groups undelivered non-copy mail by recipient, with newest mail and optional task/plan identity in public `MailTarget`. No demonstrated equivalent replacement or proof that callers can be removed. |
| `Store::turn_final_answer` | `store/history_queries.rs`: finds the last `agent_final` for a conversation/turn and hydrates content references. Raw event JSON is not an equivalent text read. |
| `Store::pending_mail_plans` | `store/mail_and_events.rs`: reads distinct conversation/plan pairs with undelivered task-role mail, across conversations. Other per-conversation mail reads do not replace this query directly. |
| `Store::count_since_user` | `store/messages_and_turns.rs`: counts an author's messages after the latest user message in a conversation (zero sequence fallback). No evidence authorizes removal of this public read. |
| `Store::last_plan_that_ran` | `store/plan_tasks.rs`: excludes `except` and proposed/revised task states, returning an optional plan. Reading the current plan does not express that selection. |
| `Store::last_plan_with_approvals` | `store/plan_tasks.rs`: selects a plan with approved work, then returns its tasks; returns an empty vector when absent. Its selection differs from `last_plan_that_ran`. |
| `Details::terminal_visible` | `tui/details.rs`: includes allocated areas and the overview terminal-only/split fallback. It does not prove attachment of a live PTY. Preserve the predicate and public access; correct the incomplete description. |
| `Theme::checkbox` | `tui/theme/palette.rs`: formats status-dependent checklist text through tone/glyph selection; it is not just a color helper. |
| `Theme::warn` | `tui/theme/palette.rs`: yellow when colored, plain when monochrome. `warning()` adds bold in monochrome mode, so aliasing the two would change that result. |
| `Theme::purple` | `tui/theme/palette.rs`: public purple style token, respecting monochrome selection. No evidence establishes that external users can discard it. |

The integration regressions compile all ten methods through public exports.
They protect availability, the clarified terminal visibility description and
Theme mode distinctions. They do not establish every historical SQL detail as a
newly supported contract. Existing Store, Details and theme tests remain the
behavior checks. Test calls added for availability are not evidence of production
demand. No signatures, implementations, aliases or deprecation attributes change.

## Individual image decisions

| Retained file | Inspected content and identity |
|---|---|
| [`agent-layout-1.png`](../work/visualizations/agent-layout-1.png) | Canvas design with task cards and inspector; Git blob `de1306d8b2a4dd7151e2c458d3cda69f9b8f267f`. |
| [`agent-workbench-pty-uniform-font.png`](../work/visualizations/agent-workbench-pty-uniform-font.png) | Workbench design with diff and PTY; Git blob `8599a11d45bf961adb19f2eb58e17775fa2d7482`. |

Before this decision, a repository text search found no literal references to
either PNG. Both are tracked and visually match their design subject; the linked
HTML designs in [the capability map](grouped-work-tui.md) provide context, not
proof of identical output or a regeneration path. Preserve both as historical
design references of unconfirmed current manual use. The `docs/images` captures
come from TestBackend buffers and are a separate source of rendering evidence.
Neither these retained PNGs nor their new documentation links prove current
runtime appearance. Their bytes are unchanged.

Revisit API removal only with reliable consumer/compatibility evidence and an
explicit decision to change the public boundary. Revisit image removal after
establishing ownership, regeneration or an explicit retirement decision. This
retention decision makes no claim that those missing facts have been supplied.

## Architecture descriptions reconciled

- `lean/mod.rs`: replace the nonexistent `docs/lean-core.md` reference with module
  ownership and the existing terminal capability map.
- `tui/mod.rs`: describe Canvas, inspector, Workbench and composer, with background
  requests/polling and local drawing; remove the unmeasured promise of instant
  interaction. `render/coordinator.rs`, `app/requests.rs` and
  `app/conversation/poller.rs` implement these boundaries.
- `Details::terminal_visible`: document both branches of the existing predicate.
- The capability map distinguishes historical validation totals and design PNGs
  from TestBackend captures and validation of later changes.
- The daemon connection/framing/local-protocol descriptions already match the
  sequential loop in `daemon/lifecycle.rs`, following `protocol-models`.
  Launch/native/MCP descriptions already reflect the removal of the empty
  settings carrier and the shared error conversion by `launch-cleanup`.
  Those completed scopes are preserved without duplicate changes here.
