//! Opt-in, test-only capture adapter for the real TUI renderer.
//!
//! Run with `ANTISLOP_CAPTURE_DIR=/absolute/new/run ./cargo.sh test --lib \
//! lean::tui::capture::export_catalog -- --exact --ignored --test-threads=1`.
//! The directory must be new; the final-renders.json object contains every
//! TestBackend cell, and manifest.json maps the captured families to section 2.

use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    io::{BufWriter, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry {
    id: &'static str,
    width: u16,
    height: u16,
}

const GEOMETRIES: &[Geometry] = &[
    Geometry {
        id: "minimum-12x5",
        width: 12,
        height: 5,
    },
    Geometry {
        id: "compact-40x12",
        width: 40,
        height: 12,
    },
    Geometry {
        id: "standard-80x24",
        width: 80,
        height: 24,
    },
    Geometry {
        id: "wide-100x30",
        width: 120,
        height: 30,
    },
    Geometry {
        id: "large-120x40",
        width: 120,
        height: 40,
    },
    Geometry {
        id: "large-160x50",
        width: 160,
        height: 50,
    },
    Geometry {
        id: "inspector-below-width",
        width: 119,
        height: 16,
    },
    Geometry {
        id: "inspector-at-width",
        width: 120,
        height: 16,
    },
    Geometry {
        id: "inspector-above-width",
        width: 121,
        height: 16,
    },
    Geometry {
        id: "inspector-below-height",
        width: 120,
        height: 15,
    },
    Geometry {
        id: "inspector-above-height",
        width: 120,
        height: 17,
    },
    Geometry {
        id: "list-below-width",
        width: 119,
        height: 30,
    },
    Geometry {
        id: "list-at-width",
        width: 120,
        height: 30,
    },
    Geometry {
        id: "list-above-width",
        width: 121,
        height: 30,
    },
    Geometry {
        id: "list-below-height",
        width: 120,
        height: 29,
    },
    Geometry {
        id: "list-above-height",
        width: 120,
        height: 31,
    },
    Geometry {
        id: "split-below-height",
        width: 80,
        height: 29,
    },
    Geometry {
        id: "split-at-height",
        width: 80,
        height: 30,
    },
    Geometry {
        id: "split-above-height",
        width: 80,
        height: 31,
    },
    Geometry {
        id: "diff-below-width",
        width: 95,
        height: 30,
    },
    Geometry {
        id: "diff-at-width",
        width: 96,
        height: 30,
    },
    Geometry {
        id: "diff-above-width",
        width: 97,
        height: 30,
    },
];

const BASE_GEOMETRY_COUNT: usize = 6;

#[derive(Clone, Copy)]
struct Fixture {
    id: &'static str,
    view: &'static str,
    state: &'static str,
}

const SECTION_2_CATALOG: &[(&str, &[&str])] = &[
    (
        "01 graph D1/D2",
        &[
            "graph-ordinary",
            "graph-empty",
            "graph-compact",
            "graph-waiting",
            "graph-error",
        ],
    ),
    (
        "02 Workbench agent and session shell",
        &[
            "workbench-summary",
            "summary-compact",
            "workbench-agent",
            "native-pty",
            "native-pty-error",
            "native-pty-empty-history",
            "shell-session-empty",
            "session-completion",
        ],
    ),
    (
        "03 Workbench Summary tab",
        &[
            "workbench-summary",
            "summary-compact",
            "session-completion",
            "summary-empty",
            "summary-waiting",
            "recovery-summary",
            "recovery-integrated",
        ],
    ),
    (
        "04 Workbench Conversation tab",
        &[
            "workbench-tab-conversation",
            "conversation-compact",
            "conversation-no-history",
        ],
    ),
    (
        "05 Workbench Results tab",
        &[
            "workbench-tab-results",
            "results-compact",
            "results-empty",
            "results-waiting",
            "results-error",
        ],
    ),
    (
        "06 Workbench Checks tab",
        &[
            "workbench-tab-checks",
            "checks-compact",
            "checks-empty",
            "checks-waiting",
            "checks-error",
        ],
    ),
    (
        "07 Workbench Comments tab",
        &[
            "workbench-tab-comments",
            "comments-compact",
            "comments-empty",
        ],
    ),
    (
        "08 Workbench Diff tab",
        &[
            "workbench-tab-diff",
            "diff-compact",
            "diff-empty",
            "diff-waiting",
            "diff-error",
            "diff-no-history",
        ],
    ),
    (
        "09 Plan and plan history",
        &[
            "plan-view",
            "plan-history-ordinary",
            "plan-history-loading",
            "plan-history-empty",
            "plan-history-error",
        ],
    ),
    (
        "10 Configuration",
        &[
            "config-error",
            "config-ordinary",
            "config-empty",
            "config-waiting",
            "config-compact",
        ],
    ),
    (
        "11 Decision inbox",
        &[
            "decision-inbox",
            "decision-inbox-empty",
            "decision-inbox-error",
        ],
    ),
    (
        "12 Message composer",
        &[
            "workbench-chat-empty",
            "send-empty",
            "send-pending",
            "send-error",
            "composer-attachment",
            "composer-broadcast",
            "composer-steer",
            "composer-interrupt",
        ],
    ),
    (
        "13 Comment, rejection, and recovery editors",
        &[
            "comment-editor",
            "comment-editor-empty",
            "reject-reason-editor",
            "recovery-guidance-editor",
        ],
    ),
    (
        "14 Question and scope confirmation",
        &["interview-question", "interview-scope", "interview-error"],
    ),
    ("15 CLI consent", &["cli-approval"]),
    (
        "16 Stopped task actions and recovery",
        &["unblock", "unblock-manual-fix", "unblock-error"],
    ),
    (
        "17 Flow editor",
        &[
            "flow-editor",
            "flow-editor-empty",
            "flow-editor-waiting",
            "flow-editor-error",
        ],
    ),
    (
        "18 Palette and existing picker variants",
        &[
            "palette",
            "palette-empty",
            "new-session",
            "palette-projects",
            "palette-sessions",
            "session-list-loading",
            "session-list-error",
            "session-delete-pending",
            "session-delete-error",
            "palette-agents",
            "palette-recipients",
            "palette-history",
            "palette-slash",
            "rigor-small-change",
            "rigor-feature",
            "rigor-refactoring",
            "rigor-security",
            "rigor-code-review",
            "rigor-no-quality-gates",
            "rigor-research",
            "rigor-document",
        ],
    ),
    ("19 Graph filters", &["graph-filters"]),
    ("20 Contextual help", &["help"]),
    ("21 Delete confirmation", &["delete-confirmation"]),
    (
        "22 New session",
        &["new-session-empty", "new-session-composer"],
    ),
];

const FIXTURES: &[Fixture] = &[
    Fixture {
        id: "session-list-loading",
        view: "picker-sessions",
        state: "waiting",
    },
    Fixture {
        id: "session-list-error",
        view: "picker-sessions",
        state: "error",
    },
    Fixture {
        id: "session-delete-pending",
        view: "picker-sessions",
        state: "waiting",
    },
    Fixture {
        id: "session-delete-error",
        view: "picker-sessions",
        state: "error",
    },
    Fixture {
        id: "plan-history-ordinary",
        view: "plan-history",
        state: "ordinary",
    },
    Fixture {
        id: "unblock-error",
        view: "unblock",
        state: "error",
    },
    Fixture {
        id: "summary-compact",
        view: "workbench-summary",
        state: "compact",
    },
    Fixture {
        id: "conversation-compact",
        view: "workbench-conversation",
        state: "compact",
    },
    Fixture {
        id: "results-compact",
        view: "workbench-results",
        state: "compact",
    },
    Fixture {
        id: "checks-compact",
        view: "workbench-checks",
        state: "compact",
    },
    Fixture {
        id: "comments-compact",
        view: "workbench-comments",
        state: "compact",
    },
    Fixture {
        id: "diff-compact",
        view: "workbench-diff",
        state: "compact",
    },
    Fixture {
        id: "config-compact",
        view: "configuration",
        state: "compact",
    },
    Fixture {
        id: "graph-waiting",
        view: "graph",
        state: "waiting",
    },
    Fixture {
        id: "graph-error",
        view: "graph",
        state: "error",
    },
    Fixture {
        id: "session-completion",
        view: "workbench-summary",
        state: "ordinary",
    },
    Fixture {
        id: "new-session-empty",
        view: "new-session",
        state: "empty",
    },
    Fixture {
        id: "shell-session-empty",
        view: "workbench",
        state: "empty",
    },
    Fixture {
        id: "interview-error",
        view: "questions-scope",
        state: "error",
    },
    Fixture {
        id: "decision-inbox-error",
        view: "decision-inbox",
        state: "error",
    },
    Fixture {
        id: "flow-editor-empty",
        view: "flow-editor",
        state: "empty",
    },
    Fixture {
        id: "summary-empty",
        view: "workbench-summary",
        state: "empty",
    },
    Fixture {
        id: "summary-waiting",
        view: "workbench-summary",
        state: "waiting",
    },
    Fixture {
        id: "conversation-no-history",
        view: "workbench-conversation",
        state: "no_history",
    },
    Fixture {
        id: "results-empty",
        view: "workbench-results",
        state: "empty",
    },
    Fixture {
        id: "results-waiting",
        view: "workbench-results",
        state: "waiting",
    },
    Fixture {
        id: "results-error",
        view: "workbench-results",
        state: "error",
    },
    Fixture {
        id: "checks-empty",
        view: "workbench-checks",
        state: "empty",
    },
    Fixture {
        id: "checks-waiting",
        view: "workbench-checks",
        state: "waiting",
    },
    Fixture {
        id: "checks-error",
        view: "workbench-checks",
        state: "error",
    },
    Fixture {
        id: "comments-empty",
        view: "workbench-comments",
        state: "empty",
    },
    Fixture {
        id: "diff-empty",
        view: "workbench-diff",
        state: "empty",
    },
    Fixture {
        id: "diff-waiting",
        view: "workbench-diff",
        state: "waiting",
    },
    Fixture {
        id: "diff-error",
        view: "workbench-diff",
        state: "error",
    },
    Fixture {
        id: "diff-no-history",
        view: "workbench-diff",
        state: "no_history",
    },
    Fixture {
        id: "config-ordinary",
        view: "configuration",
        state: "ordinary",
    },
    Fixture {
        id: "config-empty",
        view: "configuration",
        state: "empty",
    },
    Fixture {
        id: "config-waiting",
        view: "configuration",
        state: "waiting",
    },
    Fixture {
        id: "decision-inbox-empty",
        view: "decision-inbox",
        state: "empty",
    },
    Fixture {
        id: "comment-editor-empty",
        view: "comment-editor",
        state: "empty",
    },
    Fixture {
        id: "flow-editor-waiting",
        view: "flow-editor",
        state: "waiting",
    },
    Fixture {
        id: "flow-editor-error",
        view: "flow-editor",
        state: "error",
    },
    Fixture {
        id: "palette-empty",
        view: "picker-projects",
        state: "empty",
    },
    Fixture {
        id: "graph-ordinary",
        view: "graph",
        state: "ordinary",
    },
    Fixture {
        id: "graph-empty",
        view: "graph",
        state: "empty",
    },
    Fixture {
        id: "graph-compact",
        view: "graph",
        state: "compact",
    },
    Fixture {
        id: "workbench-summary",
        view: "workbench-summary",
        state: "ordinary",
    },
    Fixture {
        id: "workbench-agent",
        view: "workbench-agent",
        state: "ordinary",
    },
    Fixture {
        id: "workbench-chat-empty",
        view: "workbench-chat",
        state: "empty",
    },
    Fixture {
        id: "decision-inbox",
        view: "decision-inbox",
        state: "waiting",
    },
    Fixture {
        id: "interview-question",
        view: "questions-scope",
        state: "waiting",
    },
    Fixture {
        id: "interview-scope",
        view: "questions-scope",
        state: "ordinary",
    },
    Fixture {
        id: "cli-approval",
        view: "cli-consent",
        state: "waiting",
    },
    Fixture {
        id: "flow-editor",
        view: "flow-editor",
        state: "ordinary",
    },
    Fixture {
        id: "graph-filters",
        view: "filters",
        state: "ordinary",
    },
    Fixture {
        id: "delete-confirmation",
        view: "delete-confirmation",
        state: "waiting",
    },
    Fixture {
        id: "new-session",
        view: "picker-sessions",
        state: "empty",
    },
    Fixture {
        id: "native-pty",
        view: "session-native-pty",
        state: "ordinary",
    },
    Fixture {
        id: "native-pty-empty-history",
        view: "session-native-pty",
        state: "no_history",
    },
    Fixture {
        id: "native-pty-error",
        view: "session-native-pty",
        state: "error",
    },
    Fixture {
        id: "send-error",
        view: "composer",
        state: "error",
    },
    Fixture {
        id: "send-pending",
        view: "composer",
        state: "waiting",
    },
    Fixture {
        id: "send-empty",
        view: "composer",
        state: "empty",
    },
    Fixture {
        id: "config-error",
        view: "configuration",
        state: "error",
    },
    Fixture {
        id: "plan-history-loading",
        view: "plan-history",
        state: "waiting",
    },
    Fixture {
        id: "plan-history-empty",
        view: "plan-history",
        state: "empty",
    },
    Fixture {
        id: "plan-history-error",
        view: "plan-history",
        state: "error",
    },
    Fixture {
        id: "help",
        view: "help",
        state: "ordinary",
    },
    Fixture {
        id: "palette",
        view: "palette",
        state: "ordinary",
    },
    Fixture {
        id: "comment-editor",
        view: "comment-editor",
        state: "ordinary",
    },
    Fixture {
        id: "unblock",
        view: "unblock",
        state: "waiting",
    },
    Fixture {
        id: "unblock-manual-fix",
        view: "unblock",
        state: "waiting",
    },
    Fixture {
        id: "recovery-summary",
        view: "workbench-summary",
        state: "error",
    },
    Fixture {
        id: "recovery-integrated",
        view: "workbench-summary",
        state: "ordinary",
    },
    Fixture {
        id: "workbench-tab-conversation",
        view: "workbench-conversation",
        state: "ordinary",
    },
    Fixture {
        id: "workbench-tab-results",
        view: "workbench-results",
        state: "ordinary",
    },
    Fixture {
        id: "workbench-tab-checks",
        view: "workbench-checks",
        state: "ordinary",
    },
    Fixture {
        id: "workbench-tab-comments",
        view: "workbench-comments",
        state: "ordinary",
    },
    Fixture {
        id: "workbench-tab-diff",
        view: "workbench-diff",
        state: "ordinary",
    },
    Fixture {
        id: "plan-view",
        view: "plan",
        state: "ordinary",
    },
    Fixture {
        id: "new-session-composer",
        view: "new-session",
        state: "ordinary",
    },
    Fixture {
        id: "composer-attachment",
        view: "composer",
        state: "ordinary",
    },
    Fixture {
        id: "composer-broadcast",
        view: "composer",
        state: "ordinary",
    },
    Fixture {
        id: "composer-steer",
        view: "composer",
        state: "ordinary",
    },
    Fixture {
        id: "composer-interrupt",
        view: "composer",
        state: "ordinary",
    },
    Fixture {
        id: "reject-reason-editor",
        view: "comment-editor",
        state: "ordinary",
    },
    Fixture {
        id: "recovery-guidance-editor",
        view: "comment-editor",
        state: "ordinary",
    },
    Fixture {
        id: "palette-projects",
        view: "picker-projects",
        state: "ordinary",
    },
    Fixture {
        id: "palette-sessions",
        view: "picker-sessions",
        state: "ordinary",
    },
    Fixture {
        id: "palette-agents",
        view: "picker-agents",
        state: "ordinary",
    },
    Fixture {
        id: "palette-recipients",
        view: "picker-recipients",
        state: "ordinary",
    },
    Fixture {
        id: "palette-history",
        view: "picker-history",
        state: "ordinary",
    },
    Fixture {
        id: "palette-slash",
        view: "picker-slash-commands",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-small-change",
        view: "rigor-picker",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-feature",
        view: "rigor-picker",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-refactoring",
        view: "rigor-picker",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-security",
        view: "rigor-picker",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-code-review",
        view: "rigor-picker",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-no-quality-gates",
        view: "rigor-picker",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-research",
        view: "rigor-picker",
        state: "ordinary",
    },
    Fixture {
        id: "rigor-document",
        view: "rigor-picker",
        state: "ordinary",
    },
];

fn apply_fixture(app: &mut App, id: &str) {
    match id {
        "summary-compact" => apply_fixture(app, "workbench-summary"),
        "conversation-compact" => apply_fixture(app, "workbench-tab-conversation"),
        "results-compact" => apply_fixture(app, "workbench-tab-results"),
        "checks-compact" => apply_fixture(app, "workbench-tab-checks"),
        "comments-compact" => apply_fixture(app, "workbench-tab-comments"),
        "diff-compact" => apply_fixture(app, "workbench-tab-diff"),
        "config-compact" => apply_fixture(app, "config-ordinary"),
        "graph-waiting" | "graph-error" => {
            apply_fixture(app, "graph-ordinary");
            app.selected = "worker:t1".into();
            let task = app
                .view
                .tasks
                .iter_mut()
                .find(|task| task.id == "t1")
                .expect("graph task");
            task.status = if id == "graph-waiting" {
                "waiting"
            } else {
                "failed"
            }
            .into();
            task.reason = Some(
                if id == "graph-waiting" {
                    "Waiting for upstream capture"
                } else {
                    "Fixture capture operation failed"
                }
                .into(),
            );
            if let Some(agent) = app.view.agents.get_mut("worker:t1") {
                agent.running = false;
            }
        }
        "session-completion" => {
            apply_fixture(app, "shell-session-empty");
            app.details.as_mut().expect("session details").scroll = usize::MAX;
            app.view.cards.push(TranscriptCard::Completion {
                sent_at: None,
                branch: Some("feature/capture".into()),
                summary: "Capture session completed".into(),
                actions: [
                    crate::lean::daemon::SessionActionKind::CreatePr,
                    crate::lean::daemon::SessionActionKind::MergeWithMain,
                    crate::lean::daemon::SessionActionKind::ContinueWork,
                ]
                .into_iter()
                .map(|kind| view::CompletionActionState { kind, result: None })
                .collect(),
                selected: 0,
                latest_result: None,
            });
        }
        "graph-ordinary" | "workbench-summary" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]},
                {"id":"t2","title":"Verify rendering","goal":"Check captured output","kind":"testing","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Capture is complete"}],"depends_on":["t1"]}
            ]}}));
            app.navigation.screen = Screen::Graph;
            app.set_focus(Focus::Graph);
            app.graph_inspector_visible = true;
            if id == "graph-ordinary" {
                app.view.tasks.insert(
                    0,
                    view::TaskRow {
                        id: "options-t1".into(),
                        title: "Choose implementation approach".into(),
                        kind: "options".into(),
                        status: "approved".into(),
                        ..Default::default()
                    },
                );
                app.view
                    .task_meta
                    .entry("options-t1".into())
                    .or_default()
                    .options_for = Some("t1".into());
                app.view.tasks[1].depends_on.push("options-t1".into());
                app.view.tasks[1].reviewer_summary = Some("Review implementation".into());
                app.view.tasks[1].status = "working".into();
                app.view.tasks[1].round = 2;
                for role in [
                    ORCHESTRATOR,
                    crate::lean::PLANNER,
                    view::PLAN_REVIEWER,
                    "worker:options-t1",
                ] {
                    app.view.agents.insert(
                        role.into(),
                        view::Agent {
                            turns: 1,
                            busy_ms: 14_000,
                            activity: "Completed".into(),
                            ..Default::default()
                        },
                    );
                }
                app.view.agents.insert(
                    "worker:t1".into(),
                    view::Agent {
                        running: true,
                        busy_ms: 82_000,
                        activity: "Checking the implementation".into(),
                        ..Default::default()
                    },
                );
                app.select_node("worker:t1".into());
            }
            if id == "workbench-summary" {
                app.navigation.screen = Screen::Workbench;
                app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
                app.details = Some(Details::new("worker:t1".into()));
            }
        }
        "graph-empty" | "graph-compact" => {
            app.navigation.screen = Screen::Graph;
            app.set_focus(Focus::Graph);
        }
        "workbench-agent" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]}
            ]}}));
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
            app.details = Some(Details::new("worker:t1".into()));
        }
        "new-session-empty" => {
            apply_fixture(app, "new-session-composer");
            let details = app.details.as_mut().expect("new session details");
            details
                .drafts
                .borrow_mut()
                .get_mut(&details.composer_key())
                .expect("session draft")
                .input = InputBuffer::with_text("");
        }
        "shell-session-empty" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Session;
            app.navigation
                .update_presentation(|presentation| presentation.tab = WorkbenchTab::Summary);
            app.details = Some(Details::new("conversation".into()));
        }
        "workbench-chat-empty" | "new-session-composer" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.focus = NavigationFocus::Composer;
            if id == "new-session-composer" {
                app.navigation.target = WorkbenchTarget::Session;
                let mut details = Details::new("orchestrator".into());
                details.conversation = app.conversation.clone();
                details.editing = true;
                let composer_key = details.composer_key();
                details.drafts.borrow_mut().insert(
                    composer_key,
                    details::Composer {
                        input: InputBuffer::with_text("Describe the goal for this new session"),
                        ..details::Composer::default()
                    },
                );
                app.details = Some(details);
            } else {
                app.input = InputBuffer::with_text("");
            }
        }
        "composer-broadcast" | "composer-steer" | "composer-interrupt" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]}
            ]}}));
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
            app.navigation.focus = NavigationFocus::Composer;
            let mut details = Details::new("worker:t1".into());
            details.conversation = app.conversation.clone();
            details.editing = true;
            let mode = match id {
                "composer-steer" => crate::lean::store::Delivery::Steer,
                "composer-interrupt" => crate::lean::store::Delivery::Interrupt,
                _ => crate::lean::store::Delivery::Queue,
            };
            details.drafts.borrow_mut().insert(
                (app.conversation.clone(), "worker:t1".into()),
                details::Composer {
                    input: InputBuffer::with_text("Please coordinate with the other agents"),
                    mode,
                    address: (id == "composer-broadcast").then(|| "all".into()),
                    ..details::Composer::default()
                },
            );
            app.details = Some(details);
        }
        "workbench-tab-conversation"
        | "conversation-no-history"
        | "workbench-tab-results"
        | "results-empty"
        | "results-waiting"
        | "results-error"
        | "workbench-tab-checks"
        | "checks-empty"
        | "checks-waiting"
        | "checks-error"
        | "workbench-tab-comments"
        | "comments-empty"
        | "workbench-tab-diff"
        | "diff-empty"
        | "diff-waiting"
        | "diff-error"
        | "diff-no-history"
        | "summary-empty"
        | "summary-waiting" => apply_workbench_fixture(app, id),
        "recovery-summary" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t4","title":"Integrate reviewed change","goal":"Preserve and integrate the approved output","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Review and integration are resolved"}],"depends_on":[]}
            ]}}));
            if let Some(task) = app.view.tasks.first_mut() {
                task.status = "needs_user".into();
                task.reason = Some("Manual changes overlap with the reviewed worker branch".into());
            }
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Agent("worker:t4".into());
            let mut details = Details::new("worker:t4".into());
            details.conversation = app.conversation.clone();
            details.task = Some(
                json!({"id":"t4","title":"Integrate reviewed change","goal":"Preserve and integrate the approved output","status":"needs_user","round":1,"change_kind":"code","reason":"Manual changes overlap with the reviewed worker branch"}),
            );
            app.details = Some(details);
        }
        "recovery-integrated" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t4","title":"Integrate reviewed change","goal":"Preserve and integrate the approved output","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Review and integration are resolved"}],"depends_on":[]}
            ]}}));
            if let Some(task) = app.view.tasks.first_mut() {
                task.status = "approved".into();
                task.integration = Some(
                    json!({"outcome":"integrated","source":"task_branch","detail":"Reviewed branch merged into the session"}),
                );
            }
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Agent("worker:t4".into());
            let mut details = Details::new("worker:t4".into());
            details.conversation = app.conversation.clone();
            details.task = Some(
                json!({"id":"t4","title":"Integrate reviewed change","goal":"Preserve and integrate the approved output","status":"approved","round":1,"change_kind":"code","integration":{"outcome":"integrated","source":"task_branch","detail":"Reviewed branch merged into the session"}}),
            );
            app.details = Some(details);
        }
        "decision-inbox" | "interview-question" | "interview-scope" => {
            let question: crate::lean::interview::Question = serde_json::from_value(json!({
                "id":"scope-confirmation","context":"Confirm the work scope.",
                "question":"Should the adapter include every remaining section 2 view?",
                "options":["Include all listed views","Limit to named surfaces"]
            }))
            .expect("fixture question deserializes");
            let mut form = interview::Form::new(42, question);
            if id == "interview-scope" {
                form.visible = false;
                form.set_scope_summary(Some(
                    "TUI capture adapter: all renderer families and six terminal geometries.",
                ));
                app.view.scope_confirmation_summary = Some(
                    "TUI capture adapter: all renderer families and six terminal geometries."
                        .into(),
                );
                app.view.user_decision = true;
            }
            app.interview_forms.insert(app.conversation.clone(), form);
            app.view.workflow_phase = Some("interview".into());
            app.view.interview_question = Some((42, serde_json::from_value(json!({
                "id":"scope-confirmation","context":"Confirm the work scope.","question":"Should the adapter include every remaining section 2 view?","options":["Include all listed views","Limit to named surfaces"]
            })).expect("fixture question deserializes")));
            if id == "decision-inbox" {
                app.open_decision_inbox();
            }
        }
        "cli-approval" => {
            app.view
                .approvals
                .push(crate::lean::tui::view::PendingApproval {
                    request: "approval-fixture-42".into(),
                    role: "worker:t1".into(),
                    provider: "codex".into(),
                    summary: "Run the focused renderer capture check".into(),
                    options: vec![
                        ("1".into(), "Allow once".into(), "allow_once".into()),
                        ("2".into(), "Deny".into(), "deny".into()),
                    ],
                    orchestrator: false,
                });
            app.open_decision_inbox();
        }
        "flow-editor" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]}
            ]}}));
            app.flow_editor = Some(FlowEditor::edit_task(&app.view, &app.view.tasks[0]));
        }
        "plan-view" => {
            apply_fixture(app, "workbench-summary");
            app.navigation.screen = Screen::Graph;
            app.details = None;
            app.view.session_branch = Some("feature/plan-review".into());
            app.view.plan = Some("p-fixture".into());
            app.view.plan_doc =
                "# Plan\n\nImplement the requested change with verified acceptance criteria."
                    .into();
            app.plan_view = Some(PlanView::new("p-fixture".into()));
        }
        "graph-filters" => app.graph_filters = Some(graph_filters::FilterMenu::default()),
        "delete-confirmation" => {
            let mut picker = Picker::new(Kind::Conversations);
            picker.conversations.push(palette::ConversationItem {
                id: "old-session".into(),
                project: app.project.clone(),
                title: "Prior review session".into(),
            });
            picker.confirm_delete = true;
            picker.delete_target = picker.conversations.first().cloned();
            app.picker = Some(picker);
        }
        "session-list-loading"
        | "session-list-error"
        | "session-delete-pending"
        | "session-delete-error" => {
            let target = palette::ConversationItem {
                id: "old-session".into(),
                project: app.project.clone(),
                title: "Prior review session".into(),
            };
            let mut picker = Picker::new(Kind::Conversations);
            picker.conversations.push(target.clone());
            picker.status = Some(
                if id.starts_with("session-delete") {
                    "Deleting session..."
                } else {
                    "Loading sessions..."
                }
                .into(),
            );
            if id.starts_with("session-delete") {
                picker.delete_target = Some(target.clone());
                app.conversation_deletions
                    .insert((target.project.clone(), target.id.clone()));
            }
            app.picker = Some(picker);
            app.conversations_request_id = 7;
            if id == "session-list-error" {
                app.apply_update(Update::Conversations {
                    state_dir: app.client.state_dir().to_string_lossy().into_owned(),
                    request_id: 7,
                    result: Err("RAW list failure".into()),
                });
            } else if id == "session-delete-error" {
                app.apply_update(Update::ConversationDeleted {
                    project: target.project,
                    id: target.id,
                    result: Err("RAW delete failure".into()),
                });
            }
        }
        "new-session" => app.picker = Some(Picker::new(Kind::Conversations)),
        "palette-projects" | "palette-sessions" | "palette-agents" | "palette-recipients"
        | "palette-history" | "palette-slash" => {
            let kind = match id {
                "palette-projects" => Kind::Projects,
                "palette-sessions" => Kind::Conversations,
                "palette-agents" => Kind::Agents,
                "palette-recipients" => Kind::Recipients,
                "palette-history" => Kind::History,
                _ => Kind::SlashCommands,
            };
            let mut picker = Picker::new(kind);
            picker.projects.push(palette::ProjectItem {
                path: "/workspace/antislop".into(),
                title: "antislop · main".into(),
            });
            picker.conversations.push(palette::ConversationItem {
                id: "session-fixture".into(),
                project: app.project.clone(),
                title: "Session fixture".into(),
            });
            app.history
                .push("Previously reviewed capture adapter".into());
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]}
            ]}}));
            app.picker = Some(picker);
        }
        "rigor-small-change"
        | "rigor-feature"
        | "rigor-refactoring"
        | "rigor-security"
        | "rigor-code-review"
        | "rigor-no-quality-gates"
        | "rigor-research"
        | "rigor-document" => {
            let team = match id {
                "rigor-small-change" => crate::lean::tasks::Team::SmallChange,
                "rigor-feature" => crate::lean::tasks::Team::Feature,
                "rigor-refactoring" => crate::lean::tasks::Team::Refactoring,
                "rigor-security" => crate::lean::tasks::Team::SecurityFix,
                "rigor-code-review" => crate::lean::tasks::Team::CodeReview,
                "rigor-no-quality-gates" => crate::lean::tasks::Team::NoQualityGates,
                "rigor-research" => crate::lean::tasks::Team::Research,
                _ => crate::lean::tasks::Team::Document,
            };
            app.view.team = Some(team);
            app.view.team_recommended = true;
            app.view.team_reason = "Selected for the deterministic render fixture".into();
            app.picker = Some(Picker::rigor(&app.view));
        }
        "native-pty-empty-history" => {
            apply_fixture(app, "native-pty");
            let details = app.details.as_mut().expect("native details");
            details.selected_round = Some(0);
            details.history_unavailable = true;
        }
        "native-pty" | "native-pty-error" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Session;
            app.navigation.focus = NavigationFocus::WorkbenchTerminal;
            app.navigation
                .update_presentation(|presentation| presentation.split = true);
            let mut details = Details::new("orchestrator".into());
            details.conversation = app.conversation.clone();
            details.native_areas[0] = Rect::new(25, 2, 50, 10);
            details.editing = false;
            app.details = Some(details);
            app.workbench_sessions
                .push(crate::lean::store::SessionInfo {
                    turn_id: "capture-turn".into(),
                    plan_id: None,
                    role: "orchestrator".into(),
                    provider: "codex".into(),
                    session_id: Some("capture-session".into()),
                    status: "running".into(),
                    error: None,
                    upto_message: 1,
                    started_ms: 0,
                    finished_ms: None,
                    pid: None,
                });
            let cells = vec![
                crate::lean::terminal::Cell {
                    x: 0,
                    y: 0,
                    text: "codex - deterministic capture fixture".into(),
                    width: 38,
                    foreground: Some([210, 220, 230]),
                    background: Some([24, 28, 36]),
                    modifiers: 1,
                },
                crate::lean::terminal::Cell {
                    x: 0,
                    y: 2,
                    text: "Please inspect the captured session state".into(),
                    width: 41,
                    foreground: Some([150, 220, 170]),
                    background: Some([24, 28, 36]),
                    modifiers: 0,
                },
                crate::lean::terminal::Cell {
                    x: 0,
                    y: 4,
                    text: "✓ approval and routing remain deterministic".into(),
                    width: 42,
                    foreground: Some([120, 180, 240]),
                    background: Some([24, 28, 36]),
                    modifiers: 0,
                },
            ];
            let screen = crate::lean::terminal::Screen {
                handle: "capture-terminal".into(),
                revision: 1,
                columns: 50,
                rows: 10,
                cursor: Some((28, 2)),
                cells,
                error: None,
            };
            app.native_views.insert(
                format!("{}:orchestrator", app.conversation),
                native_view::capture_fixture_view(
                    screen,
                    (id == "native-pty-error")
                        .then(|| "Fixture terminal attachment failed".to_owned()),
                ),
            );
        }
        "send-error" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.focus = NavigationFocus::Composer;
            app.failed_send = Some("CLI request failed: deterministic fixture error".into());
            app.input = InputBuffer::with_text("Keep this draft 👩‍💻");
        }
        "send-pending" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.focus = NavigationFocus::Composer;
            app.sending = true;
            app.pending_send = Some("Render the requested view".into());
        }
        "send-empty" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.focus = NavigationFocus::Composer;
        }
        "composer-attachment" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.focus = NavigationFocus::Composer;
            app.input = InputBuffer::with_text("Summarize this attached design note");
            app.image_drafts.insert(
                app.conversation.clone(),
                vec![std::path::PathBuf::from("/capture/architecture-note.png")],
            );
        }
        "config-error" | "config-ordinary" | "config-empty" | "config-waiting" => {
            let mut editor = config::ConfigEditor::new(
                include_str!("../../../examples/antislop.toml"),
                "fixture.toml".into(),
            )
            .expect("config fixture parses");
            if id == "config-error" {
                let field = editor.fields.first_mut().expect("config has fields");
                field.error = Some("Fixture validation error".into());
            }
            if id == "config-empty" {
                // An unsaved draft with no provider/profile entries still has scalar settings.
                editor
                    .fields
                    .retain(|field| !matches!(field.path[0].as_str(), "providers" | "profiles"));
                editor.status = "No provider or profile entries in this draft".into();
            }
            editor.pending = id == "config-waiting";
            app.config_editor = Some(editor);
        }
        "plan-history-ordinary" => {
            let mut browser = planhistory::Browser::new();
            browser.loaded(crate::lean::store::PlansPage {
                plans: vec![crate::lean::store::PlanHistory {
                    scope_id: None,
                    id: "capture-history".into(),
                    conversation_id: app.conversation.clone(),
                    created_ms: 0,
                    status: "approved".into(),
                    is_active: false,
                    tasks: vec![],
                    agents: vec![],
                    turns: vec![],
                    reports: vec![],
                    verdicts: vec![],
                }],
                offset: 0,
                limit: 1,
                total: 1,
                has_more: false,
                active_plan_id: None,
            });
            app.plan_history = Some(browser);
        }
        "plan-history-loading" | "plan-history-empty" | "plan-history-error" => {
            let mut browser = planhistory::Browser::new();
            if id == "plan-history-empty" {
                browser.loaded(crate::lean::store::PlansPage {
                    plans: vec![],
                    offset: 0,
                    limit: 1,
                    total: 0,
                    has_more: false,
                    active_plan_id: None,
                });
            }
            if id == "plan-history-error" {
                browser.loading = false;
                browser.error = Some("Fixture history read failed".into());
            }
            app.plan_history = Some(browser);
        }
        "interview-error" => {
            apply_fixture(app, "interview-question");
            let form = app
                .interview_forms
                .get_mut(&app.conversation)
                .expect("question fixture");
            form.error = Some("Fixture answer delivery failed".into());
            form.scroll_by(true, usize::MAX);
        }
        "decision-inbox-error" => {
            apply_fixture(app, "cli-approval");
            app.approval_submissions.insert(
                (app.conversation.clone(), "approval-fixture-42".into()),
                decision_inbox::ApprovalStatus::Failed {
                    operation_id: "capture-approval-operation".into(),
                    error: "Fixture approval delivery failed".into(),
                },
            );
            app.open_decision_inbox();
            assert_eq!(
                app.route_decision_inbox(crossterm::event::KeyEvent::new(
                    KeyCode::PageDown,
                    KeyModifiers::NONE
                )),
                Some(true)
            );
        }
        "flow-editor-empty" => {
            apply_fixture(app, "flow-editor");
            app.flow_editor = Some(flow_editor::FlowEditor::add_dependent(&app.view, "t1"));
        }
        "decision-inbox-empty" => {
            app.decision_inbox = Some(decision_inbox::Inbox::new(app.conversation.clone(), &[]))
        }
        "comment-editor-empty" => {
            app.editor = Some(Editor {
                target: "task:t1".into(),
                input: InputBuffer::with_text(""),
            })
        }
        "palette-empty" => app.picker = Some(Picker::new(Kind::Projects)),
        "flow-editor-waiting" | "flow-editor-error" => {
            apply_fixture(app, "flow-editor");
            let editor = app.flow_editor.as_mut().expect("flow editor fixture");
            editor.sending = id == "flow-editor-waiting";
            editor.error = (id == "flow-editor-error").then(|| "Fixture flow update failed".into());
        }
        "help" => app.help = Some(Help::Graph),
        "palette" => app.picker = Some(Picker::new(Kind::Rigor)),
        "comment-editor" => {
            app.editor = Some(Editor {
                target: "task:t1".into(),
                input: InputBuffer::with_text("Review this exact task range 👩‍💻"),
            })
        }
        "reject-reason-editor" => {
            app.editor = Some(Editor {
                target: "reject:task:t1".into(),
                input: InputBuffer::with_text(
                    "The acceptance evidence does not cover the required behavior.",
                ),
            })
        }
        "recovery-guidance-editor" => {
            app.editor = Some(Editor {
                target: "recovery:task:t4".into(),
                input: InputBuffer::with_text(
                    "Keep the existing branch and rerun the focused checks.",
                ),
            })
        }
        "unblock" | "unblock-manual-fix" | "unblock-error" => {
            app.unblock = Some(unblock::Unblock::new(
                "p-fixture",
                &crate::lean::tui::view::TaskRow {
                    id: "t4".into(),
                    title: "Integrate reviewed change".into(),
                    status: if id == "unblock-error" {
                        "failed"
                    } else {
                        "needs_user"
                    }
                    .into(),
                    reason: Some(
                        if id == "unblock-error" {
                            "Fixture worker process failed"
                        } else {
                            "Manual changes overlap with the reviewed worker branch"
                        }
                        .into(),
                    ),
                    change_kind: crate::lean::tasks::ChangeKind::Code,
                    ..crate::lean::tui::view::TaskRow::default()
                },
            ));
            if id == "unblock-manual-fix"
                && let Some(unblock) = app.unblock.as_mut()
            {
                unblock.selected = 3;
            }
        }
        _ => unreachable!("registered fixture {id}"),
    }
}

/// Sets the actual Details tab as the input handler does, including role policy.
fn apply_workbench_fixture(app: &mut App, id: &str) {
    if id == "workbench-tab-diff" {
        apply_fixture(app, "native-pty");
        if let Some(mut native) = app
            .native_views
            .remove(&format!("{}:orchestrator", app.conversation))
        {
            native_view::bind_capture_owner(&mut native, "capture-session:1");
            app.native_views
                .insert(format!("{}:worker:t1", app.conversation), native);
        }
        for session in &mut app.workbench_sessions {
            session.role = "worker:t1".into();
        }
        app.navigation.focus = NavigationFocus::WorkbenchContent;
    }
    let tab = match id {
        "workbench-tab-conversation" | "conversation-no-history" => WorkbenchTab::Conversation,
        "workbench-tab-results" | "results-empty" | "results-waiting" | "results-error" => {
            WorkbenchTab::Results
        }
        "workbench-tab-checks" | "checks-empty" | "checks-waiting" | "checks-error" => {
            WorkbenchTab::Checks
        }
        "workbench-tab-comments" | "comments-empty" => WorkbenchTab::Comments,
        "summary-empty" | "summary-waiting" => WorkbenchTab::Summary,
        _ => WorkbenchTab::Diff,
    };
    let key = if tab == WorkbenchTab::Checks {
        "reviewer:t1"
    } else {
        "worker:t1"
    };
    let spec = json!({"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Capture behavior verified"}],"depends_on":[]});
    app.view.apply(
        &json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[spec]}}),
    );
    app.navigation.screen = Screen::Workbench;
    app.navigation.target = WorkbenchTarget::Agent(key.into());
    app.navigation.update_presentation(|presentation| {
        presentation.tab = tab;
        presentation.split = id == "workbench-tab-diff";
    });
    let mut details = Details::new(key.into());
    details.conversation = app.conversation.clone();
    details.task = Some(
        json!({"id":"t1","title":"Prepare implementation","status":"working","round":1,"spec":spec}),
    );
    if id == "workbench-tab-diff" {
        details.native_areas[0] = Rect::new(35, 26, 120, 16);
        app.view.tasks[0].round = 1;
    }
    details.prepare_workbench_details(None, tab);
    match id {
        "workbench-tab-conversation" => app.view.messages.push(MessageEntry {
            id: "capture-message".into(),
            sent_at: None,
            sender: "user".into(),
            target: key.into(),
            text: "Capture dialogue t1".into(),
            state: "sent".into(),
            kind: "message".into(),
        }),
        "conversation-no-history" => details.history_unavailable = true,
        "workbench-tab-results" => {
            details.task.as_mut().unwrap()["report"] = json!("Capture result report verified")
        }
        "results-waiting" | "results-error" => {
            details.task.as_mut().unwrap()["_result_documents"] =
                json!([{"id":"capture-output","path":"capture-output.md"}]);
            details.rebuild_result_index();
            let selected = details
                .selected_result_document
                .clone()
                .expect("result document selected");
            if id == "results-waiting" {
                details.result_loading = Some((selected, details.result_selection_generation, 0));
            } else {
                details.result_error = Some((selected, "Fixture result read failed".into()));
            }
        }
        "workbench-tab-checks" | "checks-error" => {
            details.task.as_mut().unwrap()["checks"] = json!([
                {"id":"goal","state":"settled","status":"ok","note":"Capture goal verified"},
                {"id":"C1","state":"settled","status":if id == "checks-error" {"issue"} else {"ok"},"note":if id == "checks-error" {"Fixture check failed"} else {"Capture criterion verified"}}
            ]);
        }
        "checks-waiting" => details.task = None,
        "checks-empty" => details.task = Some(json!({"id":"t1","round":1})),
        "workbench-tab-comments" => {
            app.view.comments.insert(
                42,
                view::Comment {
                    id: 42,
                    plan_id: Some("p-fixture".into()),
                    target: "worker:t1".into(),
                    text: "Capture comment belongs to t1".into(),
                    status: "open".into(),
                },
            );
        }
        "summary-empty" => {
            app.view.tasks.clear();
            details.task = None;
        }
        "summary-waiting" => {
            app.view.tasks[0].status = "queued".into();
            app.view.tasks[0].reason = Some("Waiting for task dependencies".into());
            details.scroll = usize::MAX;
        }
        _ => {}
    }
    if tab == WorkbenchTab::Diff {
        let mut diff = DiffView::new("t1".into());
        match id {
            "diff-waiting" => {}
            "diff-error" => diff.load(Err("Fixture diff read failed".into())),
            "diff-empty" => diff.load(Ok(json!({"files":[]}))),
            "diff-no-history" => { details.selected_round = Some(0); }
            _ => diff.load(Ok(json!({
                "base":"0123456789abcdef", "head":"fedcba9876543210",
                "files":[{"path":"capture.rs","old_path":null,"status":"modified","added":1,"removed":1,"binary":false,
                    "lines":[{"kind":"hunk","old":1,"new":1,"text":"@@ -1 +1 @@"},
                    {"kind":"del","old":1,"new":null,"text":"old capture state"},
                    {"kind":"add","old":null,"new":1,"text":"verified capture state"}]}]
            }))),
        }
        app.diff = Some(diff);
    }
    app.details = Some(details);
}

fn workbench_identity(app: &App, fixture: Fixture) -> bool {
    if fixture.id == "session-completion" {
        return app.navigation.screen == Screen::Workbench
            && app.navigation.target == WorkbenchTarget::Session
            && app.navigation.presentation().tab == WorkbenchTab::Summary
            && app
                .details
                .as_ref()
                .is_some_and(|details| details.key == "conversation");
    }
    let expected_tab = match fixture.view {
        "workbench-conversation" => WorkbenchTab::Conversation,
        "workbench-results" => WorkbenchTab::Results,
        "workbench-checks" => WorkbenchTab::Checks,
        "workbench-comments" => WorkbenchTab::Comments,
        "workbench-diff" => WorkbenchTab::Diff,
        "workbench-summary" => WorkbenchTab::Summary,
        _ => return false,
    };
    let key = if expected_tab == WorkbenchTab::Checks {
        "reviewer:t1"
    } else {
        "worker:t1"
    };
    app.navigation.screen == Screen::Workbench
        && agent_selected(app, key)
        && app.navigation.presentation().tab == expected_tab
        && app.details.as_ref().is_some_and(|details| {
            details.key == key
                && workbench::tabs_for(key, true).contains(&expected_tab)
                && details.content_tab() == Some(expected_tab)
        })
        && (expected_tab != WorkbenchTab::Diff
            || app.diff.as_ref().is_some_and(|diff| diff.task == "t1"))
        && !app.workbench_areas.content.is_empty()
}

fn additional_state_reached(app: &App, fixture: Fixture) -> bool {
    match fixture.id {
        "graph-waiting" => app
            .view
            .tasks
            .iter()
            .find(|task| task.id == "t1")
            .is_some_and(|task| task.status == "waiting" && task.reason.is_some()),
        "graph-error" => app
            .view
            .tasks
            .iter()
            .find(|task| task.id == "t1")
            .is_some_and(|task| task.status == "failed" && task.reason.is_some()),
        "session-completion" => {
            matches!(workbench_summary::project(&app.view, "conversation"), workbench_summary::Summary::Completion(completion) if completion.actions.len() == 3 && completion.branch.as_deref() == Some("feature/capture"))
        }
        "plan-history-ordinary" => app.plan_history.as_ref().is_some_and(|browser| {
            !browser.loading && browser.total == 1 && browser.error.is_none()
        }),
        "unblock-error" => app.unblock.as_ref().is_some_and(|unblock| {
            unblock.status == "failed" && unblock.reason == "Fixture worker process failed"
        }),
        "interview-error" => app
            .interview_forms
            .get(&app.conversation)
            .is_some_and(|form| form.error.is_some()),
        "decision-inbox-error" => app
            .approval_submissions
            .get(&(app.conversation.clone(), "approval-fixture-42".into()))
            .is_some_and(|status| matches!(status, decision_inbox::ApprovalStatus::Failed { .. })),
        "flow-editor-empty" => app.flow_editor.as_ref().is_some_and(|editor| {
            editor.title_input.text.is_empty() && editor.goal_input.text.is_empty()
        }),
        "shell-session-empty" => app.view.tasks.is_empty() && app.view.messages.is_empty(),
        "new-session-empty" => app.details.as_ref().is_some_and(|details| {
            details
                .drafts
                .borrow()
                .get(&details.composer_key())
                .is_some_and(|draft| draft.input.text.is_empty())
        }),
        "summary-empty" => app.view.tasks.is_empty(),
        "summary-waiting" => app
            .view
            .tasks
            .first()
            .is_some_and(|task| task.status == "queued"),
        "conversation-no-history" => {
            app.view.messages.is_empty()
                && app
                    .details
                    .as_ref()
                    .is_some_and(|details| details.history_unavailable)
        }
        "results-empty" => app.details.as_ref().is_some_and(|details| {
            details
                .task
                .as_ref()
                .is_some_and(|task| task["report"].is_null())
        }),
        "results-waiting" => app.details.as_ref().is_some_and(|details| {
            details.result_loading.is_some() && details.result_error.is_none()
        }),
        "results-error" => app.details.as_ref().is_some_and(|details| {
            details.result_error.is_some() && details.result_loading.is_none()
        }),
        "checks-empty" => app.details.as_ref().is_some_and(|details| {
            details
                .task
                .as_ref()
                .is_some_and(|task| task.get("spec").is_none() && task.get("verdict").is_none())
        }),
        "checks-waiting" => app
            .details
            .as_ref()
            .is_some_and(|details| details.task.is_none()),
        "checks-error" => app.details.as_ref().is_some_and(|details| {
            details.task.as_ref().is_some_and(|task| {
                details::check_rows(task)
                    .iter()
                    .any(|row| row.status.as_deref() == Some("issue"))
            })
        }),
        "comments-empty" => app.view.comments.is_empty(),
        "diff-empty" | "diff-waiting" | "diff-error" => {
            app.diff.as_ref().is_some_and(|diff| diff.task == "t1")
                && !app.historical_diff_selected()
        }
        "diff-no-history" => app.historical_diff_selected(),
        "config-ordinary" => app.config_editor.as_ref().is_some_and(|editor| {
            !editor.fields.is_empty() && !editor.pending && !editor.is_dirty()
        }),
        "config-empty" => app.config_editor.as_ref().is_some_and(|editor| {
            !editor.fields.is_empty()
                && editor
                    .fields
                    .iter()
                    .all(|field| !matches!(field.path[0].as_str(), "providers" | "profiles"))
        }),
        "config-waiting" => app
            .config_editor
            .as_ref()
            .is_some_and(|editor| editor.pending),
        "decision-inbox-empty" => {
            app.decision_inbox
                .as_ref()
                .is_some_and(|inbox| inbox.selected.is_none())
                && app.view.interview_question.is_none()
                && app.view.approvals.is_empty()
        }
        "comment-editor-empty" => app
            .editor
            .as_ref()
            .is_some_and(|editor| editor.target == "task:t1" && editor.input.text.is_empty()),
        "palette-empty" => app
            .picker
            .as_ref()
            .is_some_and(|picker| picker.kind == Kind::Projects && picker.projects.is_empty()),
        "flow-editor-waiting" => app
            .flow_editor
            .as_ref()
            .is_some_and(|editor| editor.sending && editor.error.is_none()),
        "flow-editor-error" => app
            .flow_editor
            .as_ref()
            .is_some_and(|editor| !editor.sending && editor.error.is_some()),
        _ => false,
    }
}

fn state_marker(id: &str, geometry: Geometry) -> Option<&'static str> {
    let id = match id {
        "summary-compact" => "workbench-summary",
        "conversation-compact" => "workbench-tab-conversation",
        "results-compact" => "workbench-tab-results",
        "checks-compact" => "workbench-tab-checks",
        "comments-compact" => "workbench-tab-comments",
        "diff-compact" => "workbench-tab-diff",
        "config-compact" => return Some("Config:"),
        _ => id,
    };
    Some(match id {
        "session-list-loading" => "Loading",
        "session-list-error" | "session-delete-error" => "RAW",
        "session-delete-pending" => "Deleting",
        "graph-waiting" => "waiting",
        "graph-error" => "failed",
        "session-completion" => "Capture session completed",
        "flow-editor-waiting" if geometry.width == 12 => "Saving...",
        "flow-editor-waiting" => "Saving graph changes",
        "flow-editor-empty" => "add task after",
        "interview-error" => "Fixture answer delivery failed",
        "decision-inbox-error" => "Error: Fixture approval",
        "palette-empty" => "No other projects yet",
        "new-session" => "No other conversations",
        "unblock-error" => "Fixture worker process failed",
        "native-pty-empty-history" => "Historical PTY is read-only",
        "plan-history-ordinary" => "Plan capture-history",
        "plan-history-empty" => "No plan records are retained",
        "plan-history-loading" => "loading",
        "plan-history-error" => "Fixture history read failed",
        "summary-empty" => "worker",
        "shell-session-empty" => "Session",
        "workbench-tab-conversation" => "Capture dialogue t1",
        "conversation-no-history" => "No retained conversation history",
        "workbench-tab-results" => "Capture result report verified",
        "results-empty" => "Worker has not submitted a report yet",
        "results-waiting" => "Loading result document",
        "results-error" => "Could not load result document",
        "workbench-tab-checks" => "checks settled",
        "checks-empty" => "Reviewer has not started yet",
        "checks-waiting" => "Loading",
        "checks-error" => "with issues",
        "workbench-tab-comments" => "Capture comment belongs to t1",
        "comments-empty" => "No comments yet",
        "workbench-tab-diff" => "verified capture state",
        "diff-empty" => "No changes in this task",
        "diff-waiting" => "Loading task diff from daemon",
        "diff-error" => "Error loading changes",
        "diff-no-history" => "No historical diff is retained",
        "summary-waiting" => "Waiting for task dependencies",
        "config-ordinary" => "Configuration",
        "config-empty" => "Configuration",
        "config-waiting" => "Applying...",
        "decision-inbox-empty" => "No open decisions",
        "comment-editor-empty" => "Comment",
        "flow-editor-error" if geometry.width == 12 => "Err PgUp/D",
        "flow-editor-error" => "Err PgUp/Dn",
        _ => return None,
    })
}

fn color(color: ratatui::style::Color) -> String {
    format!("{color:?}")
}

fn responsive_na(fixture: Fixture, geometry: Geometry) -> Option<&'static str> {
    let Geometry { width, height, .. } = geometry;
    if fixture.state == "compact" && fixture.id != "graph-compact" {
        return if fixture.id == "config-compact" {
            (width != 12 || height != 5).then_some("The compact Configuration renderer is selected only below 40 columns or 9 rows; among the six baseline geometries only 12x5 takes that path.")
        } else {
            (width != 40 || height != 12).then_some("This compact Workbench specimen requires the 40x12 layout with neither agent list nor terminal split; other geometries retain their own data-state samples.")
        };
    }
    match fixture.id {
        "flow-editor-error"
        | "flow-editor-waiting"
        | "plan-history-loading"
        | "session-list-loading"
        | "session-list-error"
        | "session-delete-pending"
        | "session-delete-error" => None,
        "decision-inbox"
        | "cli-approval"
        | "flow-editor"
        | "graph-filters"
        | "delete-confirmation"
            if width < 40 =>
        {
            Some("Modal viewport is below the 40-column minimum for its full surface")
        }
        "native-pty" | "native-pty-empty-history" | "native-pty-error" if height < 24 => {
            Some("Workbench compact layout has no native terminal panel below 24 rows")
        }
        "graph-compact" if width != 40 || height != 12 => {
            Some("The compact graph sample is defined only at its 40x12 responsive geometry")
        }
        "new-session-composer" | "new-session-empty" if height < 24 => {
            Some("Workbench compact layout replaces the session composer below 24 rows")
        }
        _ if width == 12 && height == 5 => Some(
            "12x5 is the minimum terminal guard viewport; full surface content and semantic styling are intentionally not applicable",
        ),
        _ => None,
    }
}

fn rendering_finding(_fixture: Fixture, _geometry: Geometry) -> Option<&'static str> {
    // compact-status-repair exposes these required statuses. Contracts below now
    // require their painted markers at every geometry, including 12x5.
    None
}

/// Measure the square form border from actual painted cells, not a second layout.
fn rendered_form_rect(cells: &[Value], title_prefix: &str) -> Option<Value> {
    // Background graph panels can also have square corners. Locate the form
    // title first, then measure its own corners rather than their global union.
    let left = cells.iter().enumerate().find_map(|(index, cell)| {
        if !matches!(cell["text"].as_str(), Some("┌" | "+")) {
            return None;
        }
        let title = cells
            .iter()
            .skip(index + 1)
            .take(title_prefix.len())
            .filter_map(|c| c["text"].as_str())
            .collect::<String>();
        title.starts_with(title_prefix).then_some(cell)
    })?;
    let x = left["x"].as_u64()?;
    let y = left["y"].as_u64()?;
    let right = cells.iter().find(|c| {
        c["y"] == y
            && c["x"].as_u64().is_some_and(|v| v > x)
            && matches!(c["text"].as_str(), Some("┐" | "+"))
    })?["x"]
        .as_u64()?;
    let bottom = cells.iter().find(|c| {
        c["x"] == x
            && c["y"].as_u64().is_some_and(|v| v > y)
            && matches!(c["text"].as_str(), Some("└" | "+"))
    })?["y"]
        .as_u64()?;
    Some(json!({"x":x,"y":y,"width":right-x+1,"height":bottom-y+1}))
}

fn capture(fixture: Fixture, geometry: Geometry) -> Value {
    let Geometry {
        id: geometry_id,
        width,
        height,
    } = geometry;
    let mut app = App::test_app();
    apply_fixture(&mut app, fixture.id);
    if let Some(details) = &mut app.details {
        details.conversation = app.conversation.clone();
    }
    let native_fixture_state = app
        .native_views
        .values()
        .map(native_view::capture_fixture_state)
        .next()
        .unwrap_or((false, false, false));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("TestBackend creates");
    terminal
        .draw(|frame| app.draw(frame))
        .expect("App::draw succeeds");
    let buffer = terminal.backend().buffer();
    let header_area = app.workbench_areas.header;
    let workbench_header = buffer
        .content
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            header_area.contains(
                (
                    (*index % usize::from(width)) as u16,
                    (*index / usize::from(width)) as u16,
                )
                    .into(),
            )
        })
        .map(|(_, cell)| cell.symbol())
        .collect::<String>();
    let cells = buffer
        .content
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            json!({
                "x": index % usize::from(width), "y": index / usize::from(width),
                "text":cell.symbol(), "fg":color(cell.fg), "bg":color(cell.bg),
                "modifiers":format!("{:?}",cell.modifier),
            })
        })
        .collect::<Vec<_>>();
    let na_reason = responsive_na(fixture, geometry);
    let native_panel = fixture.view == "session-native-pty"
        && height >= 24
        && app.details.as_ref().is_some_and(|details| {
            details.native.contains_key("orchestrator") && !details.native_areas[0].is_empty()
        });
    let graph_inspector = fixture.view == "graph" && text_has_inspector(&cells);
    let diff_file_list_rendered = fixture.id == "workbench-tab-diff"
        && cells.chunks(usize::from(width)).any(|row| {
            let line = row
                .iter()
                .filter_map(|cell| cell["text"].as_str())
                .collect::<String>();
            line.contains("capture.rs")
                && line.contains("+1")
                && line.contains(if Theme::from_env().ascii {
                    "-1"
                } else {
                    "−1"
                })
        });
    let workbench_renderer_reached = app.navigation.screen == Screen::Workbench
        && app.details.is_some()
        && !app.workbench_areas.header.is_empty()
        && !app.workbench_areas.content.is_empty()
        && workbench_header.contains("antislop");
    let reached_view = match fixture.id {
        "graph-ordinary" | "graph-empty" | "graph-compact" => {
            app.navigation.screen == Screen::Graph
                && app.graph_area.is_some_and(|area| !area.is_empty())
        }
        "workbench-summary" => workbench_renderer_reached && agent_selected(&app, "worker:t1"),
        "workbench-chat-empty" | "native-pty" | "native-pty-empty-history" | "native-pty-error" => {
            app.navigation.screen == Screen::Workbench
        }
        "workbench-agent" => {
            app.navigation.screen == Screen::Workbench && agent_selected(&app, "worker:t1")
        }
        "recovery-summary" | "recovery-integrated" => {
            app.navigation.screen == Screen::Workbench && agent_selected(&app, "worker:t4")
        }
        "workbench-tab-conversation"
        | "workbench-tab-results"
        | "workbench-tab-checks"
        | "workbench-tab-comments"
        | "workbench-tab-diff" => workbench_identity(&app, fixture),
        "decision-inbox" => app.decision_inbox.is_some(),
        "interview-question" | "interview-scope" => app.view.interview_question.is_some(),
        "cli-approval" => app.decision_inbox.is_some(),
        "flow-editor" => app.flow_editor.is_some(),
        "graph-filters" => app.graph_filters.is_some(),
        "delete-confirmation" => app
            .picker
            .as_ref()
            .is_some_and(|picker| picker.confirm_delete && picker.delete_target.is_some()),
        "new-session" => app
            .picker
            .as_ref()
            .is_some_and(|picker| picker.kind == Kind::Conversations),
        "new-session-composer" => {
            workbench_renderer_reached && app.navigation.target == WorkbenchTarget::Session
        }
        "session-list-loading"
        | "session-list-error"
        | "session-delete-pending"
        | "session-delete-error" => app
            .picker
            .as_ref()
            .is_some_and(|picker| picker.kind == Kind::Conversations),
        "palette"
        | "palette-projects"
        | "palette-sessions"
        | "palette-agents"
        | "palette-recipients"
        | "palette-history"
        | "palette-slash"
        | "rigor-small-change"
        | "rigor-feature"
        | "rigor-refactoring"
        | "rigor-security"
        | "rigor-code-review"
        | "rigor-no-quality-gates"
        | "rigor-research"
        | "rigor-document" => app.picker.is_some(),
        "plan-view" => app.plan_view.is_some(),
        "plan-history-loading" | "plan-history-empty" | "plan-history-error" => {
            app.plan_history.is_some()
        }
        "config-error" => app.config_editor.is_some(),
        "help" => app.help == Some(Help::Graph),
        "comment-editor" | "reject-reason-editor" | "recovery-guidance-editor" => {
            app.editor.is_some()
        }
        "unblock" => app
            .unblock
            .as_ref()
            .is_some_and(|unblock| unblock.selected == 0),
        "unblock-manual-fix" => app
            .unblock
            .as_ref()
            .is_some_and(|unblock| unblock.selected == 3),
        "send-empty"
        | "send-pending"
        | "send-error"
        | "composer-attachment"
        | "composer-broadcast"
        | "composer-steer"
        | "composer-interrupt" => {
            app.navigation.screen == Screen::Workbench
                && app.navigation.focus == NavigationFocus::Composer
        }
        _ => match fixture.view {
            "workbench" | "new-session" => {
                app.navigation.screen == Screen::Workbench
                    && app.navigation.target == WorkbenchTarget::Session
                    && app.details.is_some()
            }
            "graph" => {
                app.navigation.screen == Screen::Graph
                    && app.graph_area.is_some_and(|area| !area.is_empty())
            }
            "questions-scope" => app.view.interview_question.is_some(),
            "configuration" => app.config_editor.is_some(),
            "decision-inbox" => app.decision_inbox.is_some(),
            "comment-editor" => app.editor.is_some(),
            "flow-editor" => app.flow_editor.is_some(),
            "plan-history" => app.plan_history.is_some(),
            "unblock" => app.unblock.is_some(),
            "picker-projects" => app
                .picker
                .as_ref()
                .is_some_and(|picker| picker.kind == Kind::Projects),
            view if view.starts_with("workbench-") => workbench_identity(&app, fixture),
            _ => false,
        },
    };
    let reached_state = match fixture.id {
        "session-list-loading" => app.picker.as_ref().is_some_and(|picker| picker.status.as_deref() == Some("Loading sessions...")),
        "session-list-error" => app.picker.as_ref().is_some_and(|picker| picker.status.as_deref() == Some("RAW list failure")),
        "session-delete-pending" => app.picker.as_ref().is_some_and(|picker| picker.status.as_deref() == Some("Deleting session...") && picker.delete_target.as_ref().is_some_and(|target| target.id == "old-session")) && !app.conversation_deletions.is_empty(),
        "session-delete-error" => app.picker.as_ref().is_some_and(|picker| picker.status.as_deref() == Some("RAW delete failure")) && app.conversation_deletions.is_empty(),
        "graph-empty" => app.view.tasks.is_empty(),
        "graph-compact" => width == 40 && height == 12 && !graph_inspector,
        "workbench-chat-empty" => app.input.text.is_empty() && app.navigation.focus == NavigationFocus::Composer,
        "decision-inbox" => app.decision_inbox.is_some(),
        "interview-question" => app.view.interview_question.as_ref().is_some_and(|(_, question)| question.id == "scope-confirmation") && !app.view.user_decision,
        "interview-scope" => app.view.user_decision && app.view.scope_confirmation_summary.is_some(),
        "cli-approval" => app.decision_inbox.is_some() && app.view.approvals.iter().any(|approval| approval.request == "approval-fixture-42" && approval.options.len() == 2),
        "flow-editor" => app.flow_editor.as_ref().is_some_and(|editor| matches!(&editor.mode, flow_editor::FlowEditMode::Edit { original_id } if original_id == "t1")),
        "graph-filters" => app.graph_filters.is_some(),
        "delete-confirmation" => app.picker.as_ref().is_some_and(|picker| picker.confirm_delete && picker.delete_target.as_ref().is_some_and(|item| item.id == "old-session")),
        "new-session" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Conversations),
        "new-session-composer" => app.navigation.focus == NavigationFocus::Composer
            && app.details.as_ref().is_some_and(|details| {
                details.editing
                    && details.drafts.borrow().get(&details.composer_key())
                        .is_some_and(|draft| draft.input.text.contains("new session"))
            }),
        "native-pty" => native_panel && native_fixture_state.0,
        "native-pty-empty-history" => !app.workbench_areas.terminal.is_empty() && app.details.as_ref().is_some_and(|details| details.selected_round == Some(0) && details.history_unavailable && !details.resume_round),
        "native-pty-error" => native_panel && native_fixture_state.2,
        "send-empty" => app.input.text.is_empty() && app.failed_send.is_none() && !app.sending,
        "send-pending" => app.sending && app.pending_send.is_some(),
        "send-error" => app.failed_send.is_some() && app.input.text.contains("Keep this draft"),
        "config-error" => app.config_editor.as_ref().is_some_and(|editor| editor.fields.iter().any(|field| field.error.is_some())),
        "help" => app.help == Some(Help::Graph),
        "plan-history-loading" => app.plan_history.as_ref().is_some_and(|browser| browser.loading),
        "plan-history-empty" => app.plan_history.as_ref().is_some_and(|browser| !browser.loading && browser.total == 0 && browser.error.is_none()),
        "plan-history-error" => app.plan_history.as_ref().is_some_and(|browser| browser.error.is_some()),
        "recovery-summary" => app.view.tasks.first().is_some_and(|task| task.status == "needs_user" && task.reason.is_some()) && agent_selected(&app, "worker:t4"),
        "recovery-integrated" => app.view.tasks.first().and_then(|task| task.integration.as_ref()).is_some_and(|value| value["outcome"] == "integrated") && agent_selected(&app, "worker:t4"),
        "unblock" | "unblock-manual-fix" => app.unblock.is_some(),
        "composer-attachment" => app.image_drafts.get(&app.conversation).is_some_and(|paths| !paths.is_empty()),
        "composer-broadcast" | "composer-steer" | "composer-interrupt" => app.details.as_ref().is_some_and(|details| details.drafts.borrow().values().any(|draft| match fixture.id { "composer-broadcast" => draft.address.as_deref() == Some("all"), "composer-steer" => draft.mode == crate::lean::store::Delivery::Steer, _ => draft.mode == crate::lean::store::Delivery::Interrupt })),
        "workbench-tab-conversation" => app.navigation.presentation().tab == WorkbenchTab::Conversation,
        "workbench-tab-results" => app.navigation.presentation().tab == WorkbenchTab::Results,
        "workbench-tab-checks" => app.navigation.presentation().tab == WorkbenchTab::Checks,
        "workbench-tab-comments" => app.navigation.presentation().tab == WorkbenchTab::Comments,
        "workbench-tab-diff" => app.navigation.presentation().tab == WorkbenchTab::Diff,
        "comment-editor" => app.editor.as_ref().is_some_and(|editor| editor.target == "task:t1"),
        "reject-reason-editor" => app.editor.as_ref().is_some_and(|editor| editor.target.starts_with("reject:")),
        "recovery-guidance-editor" => app.editor.as_ref().is_some_and(|editor| editor.target.starts_with("recovery:")),
        "palette" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "palette-projects" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Projects),
        "palette-sessions" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Conversations),
        "palette-agents" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Agents),
        "palette-recipients" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Recipients),
        "palette-history" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::History),
        "palette-slash" => app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::SlashCommands),
        "rigor-small-change" => app.view.team == Some(crate::lean::tasks::Team::SmallChange) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "rigor-feature" => app.view.team == Some(crate::lean::tasks::Team::Feature) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "rigor-refactoring" => app.view.team == Some(crate::lean::tasks::Team::Refactoring) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "rigor-security" => app.view.team == Some(crate::lean::tasks::Team::SecurityFix) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "rigor-code-review" => app.view.team == Some(crate::lean::tasks::Team::CodeReview) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "rigor-no-quality-gates" => app.view.team == Some(crate::lean::tasks::Team::NoQualityGates) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "rigor-research" => app.view.team == Some(crate::lean::tasks::Team::Research) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "rigor-document" => app.view.team == Some(crate::lean::tasks::Team::Document) && app.picker.as_ref().is_some_and(|picker| picker.kind == Kind::Rigor),
        "plan-view" => app.plan_view.is_some() && !app.view.plan_doc.is_empty(),
        "workbench-summary" => {
            !app.view.tasks.is_empty()
                && app.navigation.presentation().tab == WorkbenchTab::Summary
        }
        "workbench-agent" => !app.view.tasks.is_empty() && agent_selected(&app, "worker:t1"),
        "graph-ordinary" => !app.view.tasks.is_empty(),
        _ => additional_state_reached(&app, fixture),
    };
    let reached_state = if fixture.state == "compact" && fixture.id != "graph-compact" {
        if fixture.id == "config-compact" {
            width == 12 && height == 5 && app.config_editor.is_some()
        } else {
            width == 40
                && height == 12
                && !app.workbench_areas.split
                && !app.workbench_areas.list_visible
                && workbench_identity(&app, fixture)
        }
    } else {
        reached_state
    };
    let rendered_text = cells
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .filter_map(|cell| cell["text"].as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let rendered_text = rendered_text
        // Scrollbars can fall between the wrapped lines of a status message.
        // They are chrome, rather than words in its visibility marker.
        .replace(['┃', '│', '#', '|'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let state_marker_visible =
        state_marker(fixture.id, geometry).map(|marker| rendered_text.contains(marker));
    let mut record = json!({"schema":1,"fixture":fixture.id,"target":fixture.view,"view":fixture.view,"state":fixture.state,
        "geometry":geometry_id,
        "sample":format!("{}@{}",fixture.id,geometry_id),
        "name":format!("{}.{}.{}.{}.{}x{}",fixture.id,fixture.view,fixture.state,geometry_id,width,height),
        "width":width,"height":height,"cell_count":cells.len(),"cells":cells,
        "na_reason":na_reason,"native_panel":native_panel,
        "rendering_finding":rendering_finding(fixture,geometry),
        "flow_form_rect":fixture.view.eq("flow-editor").then(|| rendered_form_rect(&cells, " Flow")).flatten(),
        "panels":{"workbench_list_visible":app.workbench_areas.list_visible,
            "workbench_split":app.workbench_areas.split,
            "workbench_content":{"x":app.workbench_areas.content.x,"y":app.workbench_areas.content.y,"width":app.workbench_areas.content.width,"height":app.workbench_areas.content.height},
            "graph_inspector_rendered":graph_inspector,
            "diff_file_list_rendered":diff_file_list_rendered},
        "model_state":{"tasks":app.view.tasks.len(),"messages":app.view.messages.len(),"comments":app.view.comments.len(),
            "task_present":app.details.as_ref().map(|details| details.task.is_some()),
            "result_loading":app.details.as_ref().map(|details| details.result_loading.is_some()),
            "result_error":app.details.as_ref().and_then(|details| details.result_error.as_ref().map(|(_,error)|error.as_str())),
            "flow_sending":app.flow_editor.as_ref().map(|editor|editor.sending),
            "flow_error":app.flow_editor.as_ref().and_then(|editor|editor.error.as_deref()),
            "history_loading":app.plan_history.as_ref().map(|browser|browser.loading),
            "history_total":app.plan_history.as_ref().map(|browser|browser.total),
            "config_pending":app.config_editor.as_ref().map(|editor|editor.pending),
            "config_entry_fields":app.config_editor.as_ref().map(|editor|editor.fields.iter().filter(|field|matches!(field.path[0].as_str(),"providers"|"profiles")).count())},
        "identity":{"screen":format!("{:?}",app.navigation.screen),"target":format!("{:?}",app.navigation.target),
            "details_key":app.details.as_ref().map(|details| details.key.as_str()),
            "details_tab":app.details.as_ref().map(|details| details.tab),
            "selected_round":app.details.as_ref().and_then(|details| details.selected_round)},
        "active_tab":format!("{:?}",app.navigation.presentation().tab),
        "workbench_header":workbench_header,
        "reached_view":reached_view,"reached_state":reached_state,"state_marker":state_marker(fixture.id, geometry),"state_marker_visible":state_marker_visible,
        "decision_inbox":app.decision_inbox.is_some(),"flow_editor":app.flow_editor.is_some(),
        "unblock":app.unblock.is_some(),
        "task_integration":app.view.tasks.first().and_then(|task| task.integration.clone())});
    if matches!(
        fixture.id,
        "session-list-loading"
            | "session-list-error"
            | "session-delete-pending"
            | "session-delete-error"
    ) {
        record["model_state"]["picker_status"] = json!(
            app.picker
                .as_ref()
                .and_then(|picker| picker.status.as_deref())
        );
        record["model_state"]["delete_target"] = json!(app.picker.as_ref().and_then(|picker| {
            picker
                .delete_target
                .as_ref()
                .map(|target| target.id.as_str())
        }));
        record["model_state"]["deletion_pending"] = json!(!app.conversation_deletions.is_empty());
    }
    record
}

fn text_has_inspector(cells: &[Value]) -> bool {
    cells
        .iter()
        .filter_map(|cell| cell["text"].as_str())
        .collect::<String>()
        .contains("SELECTED WORK")
}

fn agent_selected(app: &App, key: &str) -> bool {
    matches!(&app.navigation.target, WorkbenchTarget::Agent(selected) if selected == key)
}

fn assert_capture_contract(record: &Value, fixture: Fixture, geometry: Geometry) {
    if matches!(
        fixture.id,
        "session-list-loading"
            | "session-list-error"
            | "session-delete-pending"
            | "session-delete-error"
    ) {
        let expected = match fixture.id {
            "session-list-loading" => "Loading sessions...",
            "session-list-error" => "RAW list failure",
            "session-delete-pending" => "Deleting session...",
            _ => "RAW delete failure",
        };
        assert_eq!(record["model_state"]["picker_status"], expected);
        assert_eq!(
            record["model_state"]["deletion_pending"],
            fixture.id == "session-delete-pending"
        );
        assert_eq!(
            record["model_state"]["delete_target"],
            if fixture.id == "session-delete-pending" {
                json!("old-session")
            } else {
                Value::Null
            }
        );
    }
    let Geometry { id, width, height } = geometry;
    assert_eq!(record["na_reason"], json!(responsive_na(fixture, geometry)));
    assert_eq!(record["fixture"], fixture.id);
    assert_eq!(record["target"], fixture.view);
    assert_eq!(record["view"], fixture.view);
    assert_eq!(record["state"], fixture.state);
    assert_eq!(record["geometry"], id);
    assert_eq!(record["sample"], format!("{}@{}", fixture.id, id));
    for (index, cell) in record["cells"].as_array().unwrap().iter().enumerate() {
        assert_eq!(cell["x"], index % usize::from(width));
        assert_eq!(cell["y"], index / usize::from(width));
        for key in ["text", "fg", "bg", "modifiers"] {
            assert!(cell[key].is_string(), "cell {index} lacks {key}");
        }
    }
    assert!(!record["name"].as_str().unwrap().is_empty());
    assert!(record["na_reason"].is_null() || !record["na_reason"].as_str().unwrap().is_empty());
    assert_eq!(record["width"], width);
    assert_eq!(record["height"], height);
    assert_eq!(
        record["cell_count"],
        usize::from(width) * usize::from(height)
    );
    if record["na_reason"].is_null() {
        assert!(
            record["reached_view"].as_bool().unwrap_or(false),
            "{} did not reach its actual view",
            fixture.id
        );
        assert!(
            record["reached_state"].as_bool().unwrap_or(false),
            "{} did not reach its declared state",
            fixture.id
        );
    }
    if matches!(fixture.id, "workbench-summary" | "new-session-composer")
        && record["na_reason"].is_null()
    {
        assert!(
            record["workbench_header"].as_str().is_some_and(|header| {
                if width >= 100 {
                    header.contains("Workbench /")
                } else {
                    header.contains(if fixture.id == "new-session-composer" {
                        "Message"
                    } else {
                        "Content"
                    })
                }
            }),
            "{} did not render the Workbench D3 header",
            fixture.id
        );
    }
    assert_eq!(
        record["cells"].as_array().unwrap().len(),
        usize::from(width) * usize::from(height)
    );
    if let Some(finding) = rendering_finding(fixture, geometry) {
        assert!(
            record["na_reason"].is_null(),
            "a production rendering limitation must not become N/A"
        );
        assert_eq!(record["rendering_finding"], finding);
        assert_eq!(
            record["state_marker_visible"], false,
            "remove the finding if the renderer now exposes its status"
        );
        if fixture.view == "flow-editor" {
            assert!(
                record["flow_form_rect"].is_object(),
                "finding lacks measured form rectangle"
            );
        }
    } else if record["na_reason"].is_null() && record["state_marker"].is_string() {
        assert_eq!(
            record["state_marker_visible"],
            true,
            "{} did not render {:?} at {}x{}: {}",
            fixture.id,
            record["state_marker"],
            width,
            height,
            record["cells"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|cell| cell["text"].as_str())
                .collect::<String>()
        );
    }
    match fixture.id {
        "flow-editor-waiting" => {
            assert_eq!(record["model_state"]["flow_sending"], true);
            assert!(record["model_state"]["flow_error"].is_null());
        }
        "flow-editor-error" => {
            assert_eq!(record["model_state"]["flow_sending"], false);
            assert_eq!(
                record["model_state"]["flow_error"],
                "Fixture flow update failed"
            );
        }
        "plan-history-loading" => assert_eq!(record["model_state"]["history_loading"], true),
        _ => {}
    }
    if fixture.id == "flow-editor-error" {
        let painted = record["cells"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|cell| cell["text"].as_str())
            .collect::<String>();
        assert!(
            painted.contains(if width == 12 {
                "Fixture"
            } else {
                "Fixture flow update failed"
            }),
            "the visible error label must accompany the actual cause"
        );
    }
    let geometry_id = geometry.id;
    let panels = &record["panels"];
    match geometry_id {
        "inspector-below-width"
        | "inspector-at-width"
        | "inspector-above-width"
        | "inspector-below-height"
        | "inspector-above-height" => {
            let expected = !matches!(
                geometry_id,
                "inspector-below-width" | "inspector-below-height"
            );
            assert_eq!(
                panels["graph_inspector_rendered"], expected,
                "inspector boundary must be checked against the rendered inspector panel"
            );
        }
        "list-below-width" | "list-at-width" | "list-above-width" | "list-below-height"
        | "list-above-height" => {
            let expected = width >= 120;
            assert_eq!(
                panels["workbench_list_visible"], expected,
                "Workbench list boundary must use the actual post-draw layout"
            );
        }
        "split-below-height" | "split-at-height" | "split-above-height" => {
            let expected = height >= 30;
            assert_eq!(
                panels["workbench_split"], expected,
                "Workbench split boundary must use the actual post-draw layout"
            );
        }
        "diff-below-width" | "diff-at-width" | "diff-above-width" => {
            assert_eq!(
                panels["workbench_content"]["width"],
                width.saturating_sub(2),
                "diff threshold sample records the real content panel width"
            );
            assert_eq!(
                panels["diff_file_list_rendered"],
                geometry_id != "diff-below-width",
                "diff file-list threshold must be reached by the loaded real diff panel"
            );
        }
        _ => {}
    }
    let text = record["cells"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|cell| cell["text"].as_str())
        .collect::<String>();
    if fixture.id == "new-session-composer" && record["na_reason"].is_null() {
        let rendered = record["cells"]
            .as_array()
            .unwrap()
            .chunks(usize::from(width))
            .map(|row| {
                row.iter()
                    .filter_map(|cell| cell["text"].as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            rendered.contains("Describe the goal for this new session"),
            "new-session composer fixture text is absent from the {}x{} rendered buffer: {rendered:?}",
            width,
            height
        );
    }
    match fixture.id {
        "decision-inbox" => {
            assert!(
                record["decision_inbox"].as_bool().unwrap(),
                "decision inbox fixture failed to reach the inbox renderer"
            );
            assert!(
                text.contains("Decisions") || record["na_reason"].is_string(),
                "decision inbox renderer not reached"
            );
        }
        "flow-editor" => {
            assert!(
                record["flow_editor"].as_bool().unwrap(),
                "flow editor fixture failed to reach the editor renderer"
            );
            assert!(
                text.contains("Flow Editor") || record["na_reason"].is_string(),
                "flow editor not reached"
            );
        }
        "unblock" => assert!(
            record["unblock"].as_bool().unwrap(),
            "stopped-task fixture failed to reach recovery actions"
        ),
        "recovery-integrated" => assert!(
            record["task_integration"]["outcome"] == "integrated",
            "integrated recovery state is not present in the task fixture"
        ),
        "workbench-tab-conversation" => assert_eq!(record["active_tab"], "Conversation"),
        "workbench-tab-results" => assert_eq!(record["active_tab"], "Results"),
        "workbench-tab-checks" => assert_eq!(record["active_tab"], "Checks"),
        "workbench-tab-comments" => assert_eq!(record["active_tab"], "Comments"),
        "workbench-tab-diff" => {
            assert_eq!(record["active_tab"], "Diff");
            if height >= 30 {
                assert!(
                    text.contains("Please inspect"),
                    "split diff must retain its live PTY"
                );
            }
        }
        "cli-approval" => assert!(
            text.contains("Decisions") || record["na_reason"].is_string(),
            "CLI approval card not reached"
        ),
        "graph-filters" => assert!(
            text.contains("Graph filters") || record["na_reason"].is_string(),
            "filter overlay not reached"
        ),
        "delete-confirmation" => assert!(
            text.contains("delete") || record["na_reason"].is_string(),
            "delete confirmation not reached"
        ),
        "native-pty" if height >= 24 => {
            assert!(
                record["native_panel"].as_bool().unwrap(),
                "native panel routing was not reached"
            );
            assert!(
                text.contains("Please inspect"),
                "native PTY fixture content not reached at {width}x{height}: {text}"
            );
        }
        "native-pty-empty-history" if height >= 24 => {
            assert_eq!(record["identity"]["selected_round"], 0);
            assert!(
                text.contains("Historical PTY is read-only"),
                "missing historical PTY availability notice"
            );
            assert!(
                !text.contains("Please inspect"),
                "an unavailable historical PTY must not display live terminal cells"
            );
        }
        "native-pty-error" if height >= 24 => {
            assert!(
                record["native_panel"].as_bool().unwrap(),
                "failed native panel route was not reached"
            );
            assert!(
                text.contains("Fixture terminal attachment failed"),
                "native attachment failure state was not rendered"
            );
        }
        _ => {}
    }
    if record["na_reason"].is_null() {
        let visible = record["cells"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|cell| {
                cell["text"]
                    .as_str()
                    .is_some_and(|text| !text.trim().is_empty())
            })
            .collect::<Vec<_>>();
        assert!(
            !visible.is_empty(),
            "{} rendered no visible content at {width}x{height}",
            fixture.id
        );
        let theme = Theme::from_env();
        let expected = expected_surface_style(fixture, &theme);
        let semantic_cells = semantic_surface_cells(record, fixture);
        let visible = semantic_cells.as_ref().unwrap_or(&visible);
        if semantic_cells.is_some() {
            for cell in visible {
                assert_eq!(
                    cell["fg"],
                    color(expected.fg.unwrap_or(ratatui::style::Color::Reset)),
                    "{} semantic element foreground",
                    fixture.id
                );
                assert_eq!(
                    cell["bg"],
                    color(expected.bg.unwrap_or(ratatui::style::Color::Reset)),
                    "{} semantic element background",
                    fixture.id
                );
                assert_eq!(
                    cell["modifiers"],
                    format!("{:?}", expected.add_modifier),
                    "{} semantic element modifiers",
                    fixture.id
                );
            }
        }
        let expected_fg = expected.fg.map(|color| format!("{color:?}"));
        let expected_bg = expected.bg.map(|color| format!("{color:?}"));
        let expected_modifiers = format!("{:?}", expected.add_modifier);
        let style_matches = |cell: &&Value| {
            if expected == ratatui::style::Style::default() {
                return cell["fg"] == "Reset"
                    && cell["bg"] == "Reset"
                    && cell["modifiers"] == "NONE";
            }
            expected_fg.as_ref().is_none_or(|fg| cell["fg"] == *fg)
                && expected_bg.as_ref().is_none_or(|bg| cell["bg"] == *bg)
                && (expected.add_modifier.is_empty()
                    || expected_modifiers.split(" | ").all(|modifier| {
                        cell["modifiers"].as_str().is_some_and(|actual| {
                            actual.split(" | ").any(|actual| actual == modifier)
                        })
                    }))
        };
        let expected_style_present = if semantic_cells.is_some() {
            visible.iter().all(style_matches)
        } else {
            visible.iter().any(style_matches)
        };
        let observed_styles = visible
            .iter()
            .map(|cell| {
                format!(
                    "fg={} bg={} modifiers={}",
                    cell["fg"].as_str().unwrap_or_default(),
                    cell["bg"].as_str().unwrap_or_default(),
                    cell["modifiers"].as_str().unwrap_or_default()
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        let observed_styles = observed_styles.into_iter().take(12).collect::<Vec<_>>();
        assert!(
            expected_style_present,
            "{} did not render its expected {:?} style at {width}x{height}; expected fg={expected_fg:?}, bg={expected_bg:?}, modifiers={expected_modifiers}; observed styles: {observed_styles:?}",
            fixture.id, expected
        );
    }
}

/// Select the painted semantic element, never unrelated document/native cells.
fn semantic_surface_cells(record: &Value, fixture: Fixture) -> Option<Vec<&Value>> {
    let cells = record["cells"].as_array().unwrap();
    if matches!(fixture.id, "flow-editor-empty" | "graph-filters") {
        let rect = if fixture.id == "graph-filters" {
            rendered_form_rect(cells, " Graph filters").expect("painted filter panel")
        } else {
            record["flow_form_rect"].clone()
        };
        let x = rect["x"].as_u64().expect("painted form x");
        let y = rect["y"].as_u64().expect("painted form y");
        return Some(vec![
            cells.iter().find(|c| c["x"] == x && c["y"] == y).unwrap(),
        ]);
    }
    let marker = match fixture.id {
        "flow-editor-waiting" => record["state_marker"].as_str().unwrap(),
        "flow-editor-error" => record["state_marker"].as_str().unwrap(),
        "unblock-error" => "Fixture worker process failed",
        "plan-history-loading" => "loading",
        _ => return None,
    };
    let width = record["width"].as_u64().unwrap() as usize;
    for row in cells.chunks(width) {
        for start in 0..row.len() {
            let mut text = String::new();
            for end in start..row.len() {
                text.push_str(row[end]["text"].as_str().unwrap());
                if text == marker {
                    return Some(row[start..=end].iter().collect());
                }
                if !marker.starts_with(&text) {
                    break;
                }
            }
        }
    }
    panic!("{} lacks painted semantic marker {marker:?}", fixture.id);
}

fn expected_surface_style(fixture: Fixture, theme: &Theme) -> ratatui::style::Style {
    if matches!(
        fixture.id,
        "palette-empty" | "new-session" | "workbench-chat-empty" | "send-pending" | "send-empty"
    ) {
        return theme.dim();
    }
    if matches!(
        fixture.id,
        "flow-editor-empty" | "flow-editor-waiting" | "flow-editor-error"
    ) {
        return match fixture.id {
            "flow-editor-waiting" => theme.accent(),
            "flow-editor-error" => theme.error(),
            _ => theme.separator(true),
        };
    }
    match fixture.view {
        "decision-inbox" => theme.card(),
        "questions-scope" => theme.title(),
        "cli-consent" => theme.card(),
        "composer" => theme.dim(),
        "flow-editor" => ratatui::style::Style::default()
            .add_modifier(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::REVERSED),
        "filters" => theme.separator(true),
        "new-session" => theme.dim(),
        "palette"
        | "picker-projects"
        | "picker-sessions"
        | "picker-agents"
        | "picker-recipients"
        | "picker-history"
        | "picker-slash-commands"
        | "rigor-picker"
        | "delete-confirmation" => theme.selected(),
        "unblock" => theme.error(),
        "graph" => theme.panel(),
        "workbench"
        | "workbench-agent"
        | "workbench-chat"
        | "workbench-summary"
        | "workbench-conversation"
        | "workbench-results"
        | "workbench-checks"
        | "workbench-comments"
        | "workbench-diff"
        | "session-native-pty" => theme.card(),
        "configuration" => theme.title(),
        // History navigation uses the same flat canvas as the plan shell.
        "plan-history" if fixture.id == "plan-history-loading" => {
            theme.overlay().patch(theme.accent())
        }
        "plan-history" => theme.accent(),
        "comment-editor" => theme.accent(),
        "plan" => theme.accent(),
        "help" => theme.title(),
        other => panic!("no semantic style contract for fixture view {other}"),
    }
}

fn geometries_for(fixture: Fixture) -> Vec<Geometry> {
    let mut geometries = GEOMETRIES[..BASE_GEOMETRY_COUNT].to_vec();
    let boundary_prefix = match fixture.id {
        "graph-ordinary" => Some("inspector-"),
        "workbench-summary" => Some("list-"),
        "native-pty" => Some("split-"),
        "workbench-tab-diff" => Some("diff-"),
        _ => None,
    };
    if let Some(prefix) = boundary_prefix {
        geometries.extend(
            GEOMETRIES
                .iter()
                .copied()
                .filter(|geometry| geometry.id.starts_with(prefix)),
        );
    }
    geometries
}

fn selected_samples() -> Vec<(Fixture, Geometry)> {
    let requested = std::env::var("ANTISLOP_CAPTURE_SELECT").ok();
    let all = FIXTURES
        .iter()
        .flat_map(|fixture| {
            geometries_for(*fixture)
                .into_iter()
                .map(|geometry| (*fixture, geometry))
        })
        .collect::<Vec<_>>();
    let Some(requested) = requested else {
        return all;
    };
    let mut selected = Vec::new();
    for key in requested
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        let (fixture_id, geometry_id) = key
            .split_once('@')
            .unwrap_or_else(|| panic!("capture selection must be fixture@geometry: {key}"));
        let fixture = FIXTURES
            .iter()
            .find(|fixture| fixture.id == fixture_id)
            .unwrap_or_else(|| panic!("unknown capture fixture in selection: {fixture_id}"));
        let geometry = geometries_for(*fixture)
            .into_iter()
            .find(|geometry| geometry.id == geometry_id)
            .unwrap_or_else(|| {
                panic!("geometry {geometry_id} is not applicable to fixture {fixture_id}")
            });
        assert!(
            !selected.iter().any(
                |(old_fixture, old_geometry): &(Fixture, Geometry)| old_fixture.id == fixture.id
                    && old_geometry.id == geometry.id
            ),
            "duplicate capture selection: {key}"
        );
        selected.push((*fixture, geometry));
    }
    assert!(
        !selected.is_empty(),
        "ANTISLOP_CAPTURE_SELECT selected no samples"
    );
    selected
}

const CATALOG_STATES: [&str; 6] = [
    "ordinary",
    "empty",
    "waiting",
    "error",
    "no_history",
    "compact",
];

/// Absence of a fixture never decides applicability. Each row describes the
/// surface's own model; child surfaces (tabs, PTY, composer) have their own rows.
fn semantic_na(catalog: usize, state: &str) -> Option<&'static str> {
    if state == "compact" && !matches!(catalog, 0..=7 | 9) {
        return Some(
            "Compact is a responsive geometry, not a separate data state of this surface. Its applicable data states are rendered at compact-40x12; responsive exclusions are recorded separately.",
        );
    }
    match (catalog, state) {
        (0, "no_history") => Some(
            "Graph projects the current task graph; retained conversation and historical plans belong to their own views.",
        ),
        (1, "waiting") => Some(
            "The shell owns target and layout only; operation waiting belongs to its tabs, composer, and terminal. This row does not alias a tab's loading state.",
        ),
        (2, "no_history") => Some(
            "Summary projects current status, recovery, and completion. Retained dialogue and historical diff availability belong to separate tabs.",
        ),
        (3, "empty") => Some(
            "The Conversation renderer defines an absent retained transcript as no_history, not an independently distinguishable empty state.",
        ),
        (3, "waiting" | "error") => Some(
            "Conversation projects retained messages synchronously; it has no own fetch progress or error renderer. Delivery failures belong to the composer.",
        ),
        (4..=6, "no_history") => Some(
            "Results, Checks and Comments use their own absent-data state; they do not implement a separate pruned-history body. Round metadata alone is not such a state.",
        ),
        (6, "waiting" | "error") => Some(
            "Comments reads the retained scoped comment collection synchronously; no loading or failure body exists in this tab.",
        ),
        (8, "no_history") => Some(
            "No retained plan records is the existing empty history state; there is no distinct second no-history renderer.",
        ),
        (9, "no_history") => Some(
            "Configuration edits a current draft; historical session retention is not part of its model.",
        ),
        (10, "ordinary") => Some(
            "An open decision is intrinsically waiting for an answer; the settled state closes the decision rather than becoming an ordinary decision.",
        ),
        (10, "no_history") => Some("Inbox displays open decisions, not retained decision history."),
        (11, "no_history") => Some(
            "Composer owns an unsent draft and delivery status; retained conversation belongs to the Conversation tab.",
        ),
        (12, "waiting" | "error") => Some(
            "The comment/rejection/recovery text overlay owns target and input only; transport status is outside this Editor model.",
        ),
        (12, "no_history") => Some("An input editor does not own retained history."),
        (13, "empty") => Some(
            "A question or scope confirmation requires an actual question. An empty answer is part of the waiting fixture, not an empty question surface.",
        ),
        (13, "no_history") => {
            Some("Only the current unanswered question is displayed by the form.")
        }
        (14, "ordinary" | "empty" | "error" | "no_history") => Some(
            "The CLI consent card requires a pending approval and its choices. It owns no ordinary/empty/history or error state; submission failures are rendered in the Decision inbox.",
        ),
        (15, "ordinary" | "empty" | "no_history") => Some(
            "Recovery actions require a concrete stopped task and await a choice. No task means no recovery surface; integrated results belong to Summary, not this dialog.",
        ),
        (16, "no_history") => {
            Some("Flow editor edits the current task contract, not retained history.")
        }
        (17, "no_history") => Some(
            "Session lists expose asynchronous fetch/deletion status, but no distinct history-availability flag. An empty history picker is the empty state.",
        ),
        (18, "empty" | "waiting" | "error" | "no_history") => Some(
            "Graph filters are a fixed set of local toggles; no empty collection, request lifecycle, or historical data is owned by this menu.",
        ),
        (19, "empty" | "waiting" | "error" | "no_history") => Some(
            "Contextual help is static local content selected by context, without an empty, asynchronous, or historical state.",
        ),
        (20, "ordinary" | "empty" | "error" | "no_history") => Some(
            "Delete confirmation exists only while waiting for a choice about a selected session; absence of a target closes it. It does not own deletion transport status.",
        ),
        (21, "waiting" | "error" | "no_history") => Some(
            "New session owns the initial goal draft before sending. Sending/errors belong to Message composer, and a new session has no retained history to lose.",
        ),
        _ => None,
    }
}

fn catalog_owns_fixture(catalog: usize, fixture: Fixture) -> bool {
    let views: &[&[&str]] = &[
        &["graph"],
        &[
            "workbench",
            "workbench-summary",
            "workbench-agent",
            "session-native-pty",
        ],
        &["workbench-summary"],
        &["workbench-conversation"],
        &["workbench-results"],
        &["workbench-checks"],
        &["workbench-comments"],
        &["workbench-diff"],
        &["plan", "plan-history"],
        &["configuration"],
        &["decision-inbox"],
        &["composer", "workbench-chat"],
        &["comment-editor"],
        &["questions-scope"],
        &["cli-consent"],
        &["unblock"],
        &["flow-editor"],
        &[
            "palette",
            "picker-projects",
            "picker-sessions",
            "picker-agents",
            "picker-recipients",
            "picker-history",
            "picker-slash-commands",
            "rigor-picker",
        ],
        &["filters"],
        &["help"],
        &["delete-confirmation"],
        &["new-session"],
    ];
    views[catalog].contains(&fixture.view)
}

fn combination_index(samples: &[(Fixture, Geometry)]) -> Vec<Value> {
    SECTION_2_CATALOG.iter().enumerate().flat_map(|(catalog, (view, ids))| {
        CATALOG_STATES.into_iter().flat_map(move |state| {
            let fixtures = FIXTURES.iter().filter(|fixture| ids.contains(&fixture.id) && fixture.state == state).collect::<Vec<_>>();
            assert!(fixtures.iter().all(|fixture| catalog_owns_fixture(catalog, **fixture)), "catalog {view} aliases another surface");
            let reason = semantic_na(catalog, state);
            assert_eq!(fixtures.is_empty(), reason.is_some(), "catalog {view}, state {state}: semantic policy and fixture registration disagree");
            GEOMETRIES[..BASE_GEOMETRY_COUNT].iter().map(move |geometry| {
                let captures = fixtures.iter().map(|fixture| {
                    let responsive = responsive_na(**fixture, *geometry);
                    let selected = samples.iter().any(|(f,g)| f.id == fixture.id && g == geometry);
                    json!({"fixture":fixture.id,"view":fixture.view,"state":fixture.state,
                        "sample":format!("{}@{}",fixture.id,geometry.id),
                        "status": if responsive.is_some() {"responsive_na"} else if !selected {"not_selected"} else if rendering_finding(**fixture,*geometry).is_some() {"captured_with_finding"} else {"captured"},
                        "reason":responsive,"rendering_finding":rendering_finding(**fixture,*geometry)})
                }).collect::<Vec<_>>();
                json!({"catalog":view,"state":state,"geometry":geometry.id,
                    "width":geometry.width,"height":geometry.height,"semantic_na":reason,"samples":captures})
            })
        })
    }).collect()
}

#[test]
fn fixture_contracts_reach_the_requested_views_and_export_full_buffers() {
    assert_eq!(SECTION_2_CATALOG.len(), 22);
    let fixture_ids = FIXTURES
        .iter()
        .map(|fixture| fixture.id)
        .collect::<std::collections::HashSet<_>>();
    let catalog_fixture_ids = SECTION_2_CATALOG
        .iter()
        .flat_map(|(_, fixture_ids)| fixture_ids.iter().copied())
        .collect::<std::collections::HashSet<_>>();
    assert!(
        catalog_fixture_ids
            .iter()
            .all(|id| fixture_ids.contains(id)),
        "catalog references an unregistered fixture"
    );
    assert!(
        fixture_ids
            .iter()
            .all(|id| catalog_fixture_ids.contains(id)),
        "fixture is omitted from the section 2 map"
    );
    let samples = selected_samples();
    assert_eq!(combination_index(&samples).len(), 22 * 6 * 6);
    for (fixture, geometry) in samples {
        let record = capture(fixture, geometry);
        assert_capture_contract(&record, fixture, geometry);
    }
}

#[test]
fn semantic_contracts_reject_corrupted_capture_evidence() {
    for id in [
        "session-list-loading",
        "session-list-error",
        "session-delete-pending",
        "session-delete-error",
    ] {
        let fixture = *FIXTURES.iter().find(|fixture| fixture.id == id).unwrap();
        let geometry = GEOMETRIES[2];
        let mut corrupted = capture(fixture, geometry);
        assert_capture_contract(&corrupted, fixture, geometry);
        corrupted["model_state"]["picker_status"] = json!("invented status");
        assert!(
            std::panic::catch_unwind(|| assert_capture_contract(&corrupted, fixture, geometry))
                .is_err()
        );
    }
    let fixture = *FIXTURES.iter().find(|f| f.id == "unblock-error").unwrap();
    let geometry = GEOMETRIES[1];
    let record = capture(fixture, geometry);
    assert_capture_contract(&record, fixture, geometry);
    let reject = |bad: Value| {
        assert!(
            std::panic::catch_unwind(|| assert_capture_contract(&bad, fixture, geometry)).is_err()
        );
    };
    // An unrelated correctly styled cell cannot rescue a warning-colored reason.
    let mut bad = record.clone();
    let coordinates = semantic_surface_cells(&record, fixture)
        .unwrap()
        .iter()
        .map(|c| (c["x"].clone(), c["y"].clone()))
        .collect::<Vec<_>>();
    for cell in bad["cells"].as_array_mut().unwrap() {
        if coordinates
            .iter()
            .any(|(x, y)| cell["x"] == *x && cell["y"] == *y)
        {
            cell["fg"] = json!("Rgb(255, 180, 84)");
            cell["modifiers"] = json!("NONE");
        }
    }
    let unrelated = bad["cells"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|cell| {
            !coordinates
                .iter()
                .any(|(x, y)| cell["x"] == *x && cell["y"] == *y)
        })
        .unwrap();
    let error = Theme::from_env().error();
    unrelated["text"] = json!("X");
    unrelated["fg"] = json!(color(error.fg.unwrap_or(ratatui::style::Color::Reset)));
    unrelated["bg"] = json!("Reset");
    unrelated["modifiers"] = json!(format!("{:?}", error.add_modifier));
    reject(bad);
    for (key, value) in [
        ("reached_state", json!(false)),
        ("reached_view", json!(false)),
        ("state_marker_visible", json!(false)),
        ("width", json!(41)),
        ("target", json!("plan-history")),
        ("sample", json!("wrong@compact-40x12")),
        ("na_reason", json!("invented responsive exclusion")),
    ] {
        let mut bad = record.clone();
        bad[key] = value;
        reject(bad);
    }
    let mut bad = record.clone();
    bad["cells"].as_array_mut().unwrap().pop();
    reject(bad);
    // Even forged marker metadata cannot hide removal from the actual buffer.
    let mut bad = record.clone();
    for cell in bad["cells"].as_array_mut().unwrap() {
        if coordinates
            .iter()
            .any(|(x, y)| cell["x"] == *x && cell["y"] == *y)
        {
            cell["text"] = json!(" ");
        }
    }
    reject(bad);
}

/// Serialize one production buffer at a time; memory never holds the catalog's
/// complete cell tree. The wire format remains the original cells-v1 object.
struct RenderSamples<'a>(&'a [(Fixture, Geometry)]);

impl serde::Serialize for RenderSamples<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for &(fixture, geometry) in self.0 {
            let record = capture(fixture, geometry);
            assert_capture_contract(&record, fixture, geometry);
            sequence.serialize_element(&record)?;
        }
        sequence.end()
    }
}

#[derive(serde::Serialize)]
struct RenderCatalog<'a> {
    format: &'static str,
    renders: RenderSamples<'a>,
    source: &'static str,
    states_note: &'static str,
}

#[test]
#[ignore = "writes a new durable capture run; set ANTISLOP_CAPTURE_DIR to a new directory"]
fn export_catalog() {
    let directory = std::env::var_os("ANTISLOP_CAPTURE_DIR")
        .expect("set ANTISLOP_CAPTURE_DIR to a new run directory");
    let path = Path::new(&directory);
    std::fs::create_dir(path).expect("capture output directory must not already exist");
    let samples = selected_samples();
    let combinations = combination_index(&samples);
    let mut file = BufWriter::new(
        std::fs::File::create(path.join("final-renders.json"))
            .expect("create final render catalog"),
    );
    serde_json::to_writer_pretty(&mut file, &RenderCatalog {
        format: "antislop-ratatui-testbackend-cells-v1",
        renders: RenderSamples(&samples),
        source: "App::draw on ratatui::Terminal<TestBackend>",
        states_note: "Fixture-level ordinary, empty, waiting, error, no_history, and compact states are separate real App fixtures where applicable. Each record includes na_reason for unavailable responsive states or surfaces. PTY panels use deterministic native terminal snapshots; no CLI provider starts.",
    }).expect("serialize capture catalog");
    file.flush().expect("flush capture catalog");
    let mut manifest = BufWriter::new(
        std::fs::File::create(path.join("manifest.json")).expect("create capture manifest"),
    );
    serde_json::to_writer_pretty(
        &mut manifest,
        &json!({
            "format":"antislop-ratatui-testbackend-cells-v1",
            "source":"App::draw on ratatui::Terminal<TestBackend>",
            "schema":1,
            "combinations":combinations,
            "geometries":GEOMETRIES.iter().map(|g| json!({"id":g.id,"width":g.width,"height":g.height})).collect::<Vec<_>>(),
            "fixtures":FIXTURES.iter().map(|f| json!({"fixture":f.id,"view":f.view,"state":f.state})).collect::<Vec<_>>(),
            "section_2_catalog":SECTION_2_CATALOG.iter().map(|(view,fixtures)| json!({"view":view,"fixtures":fixtures})).collect::<Vec<_>>(),
            "geometry_applicability":[
                {"geometries":["inspector-below-width","inspector-at-width","inspector-above-width","inspector-below-height","inspector-above-height"],"fixtures":["graph-ordinary"],"not_applicable_reason":"The inspector threshold belongs to the Graph renderer; other targets have no graph inspector panel."},
                {"geometries":["list-below-width","list-at-width","list-above-width","list-below-height","list-above-height"],"fixtures":["workbench-summary"],"not_applicable_reason":"The 120-column target is the Workbench agent list; other surfaces do not render that list."},
                {"geometries":["split-below-height","split-at-height","split-above-height"],"fixtures":["native-pty"],"not_applicable_reason":"The split threshold is exercised by a session with a real native terminal panel; other surfaces do not own that panel."},
                {"geometries":["diff-below-width","diff-at-width","diff-above-width"],"fixtures":["workbench-tab-diff"],"not_applicable_reason":"The 90-column threshold applies to the loaded Workbench Diff content panel; other tabs have no changed-file list."}
            ],
            "not_applicable":[
                {"item":"native OS clipboard integration","reason":"App::draw capture does not read or write the host clipboard; the clipboard is an external interaction surface, not a rendered view."},
                {"item":"external CLI broadcast/steer/interrupt execution","reason":"No CLI provider or daemon is started by a deterministic renderer capture. Rendered recipient and approval states are captured; transport behavior remains covered by existing tests."}
                ,{"item":"persisted manual repair and branch integration effects","reason":"The visible recovery choice, manual-fix selection, and integrated summary are separate captured App states. Applying them mutates daemon and worktree state, which is outside this render-only adapter."}
            ]
        }),
    ).expect("write capture manifest");
    manifest.flush().expect("flush capture manifest");
}
