//! Opt-in, test-only capture adapter for the real TUI renderer.
//!
//! Run with `ANTISLOP_CAPTURE_DIR=/absolute/new/run ./cargo.sh test --lib \
//! lean::tui::capture::export_catalog -- --exact --ignored --test-threads=1`.
//! The directory must be new; the final-renders.json object contains every
//! TestBackend cell, and manifest.json maps the captured families to section 2.

use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{io::Write, path::Path};

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
        width: 100,
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
        width: 164,
        height: 19,
    },
    Geometry {
        id: "inspector-at-width",
        width: 166,
        height: 19,
    },
    Geometry {
        id: "inspector-above-width",
        width: 168,
        height: 19,
    },
    Geometry {
        id: "inspector-below-height",
        width: 166,
        height: 18,
    },
    Geometry {
        id: "inspector-above-height",
        width: 166,
        height: 20,
    },
    Geometry {
        id: "list-below-width",
        width: 99,
        height: 24,
    },
    Geometry {
        id: "list-at-width",
        width: 100,
        height: 24,
    },
    Geometry {
        id: "list-above-width",
        width: 101,
        height: 24,
    },
    Geometry {
        id: "list-below-height",
        width: 100,
        height: 23,
    },
    Geometry {
        id: "list-above-height",
        width: 100,
        height: 25,
    },
    Geometry {
        id: "split-below-height",
        width: 80,
        height: 23,
    },
    Geometry {
        id: "split-at-height",
        width: 80,
        height: 24,
    },
    Geometry {
        id: "split-above-height",
        width: 80,
        height: 25,
    },
    Geometry {
        id: "diff-below-width",
        width: 89,
        height: 24,
    },
    Geometry {
        id: "diff-at-width",
        width: 90,
        height: 24,
    },
    Geometry {
        id: "diff-above-width",
        width: 91,
        height: 24,
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
        &["graph-ordinary", "graph-empty", "graph-compact"],
    ),
    (
        "02 Workbench agent and session shell",
        &[
            "workbench-summary",
            "workbench-agent",
            "native-pty",
            "native-pty-error",
            "native-pty-empty-history",
        ],
    ),
    (
        "03 Workbench Summary tab",
        &["workbench-summary", "recovery-summary"],
    ),
    (
        "04 Workbench Conversation tab",
        &["workbench-tab-conversation"],
    ),
    ("05 Workbench Results tab", &["workbench-tab-results"]),
    ("06 Workbench Checks tab", &["workbench-tab-checks"]),
    ("07 Workbench Comments tab", &["workbench-tab-comments"]),
    ("08 Workbench Diff tab", &["workbench-tab-diff"]),
    (
        "09 Plan and plan history",
        &[
            "plan-view",
            "plan-history-loading",
            "plan-history-empty",
            "plan-history-error",
        ],
    ),
    ("10 Configuration", &["config-error"]),
    ("11 Decision inbox", &["decision-inbox"]),
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
            "reject-reason-editor",
            "recovery-guidance-editor",
        ],
    ),
    (
        "14 Question and scope confirmation",
        &["interview-question", "interview-scope"],
    ),
    ("15 CLI consent", &["cli-approval"]),
    (
        "16 Stopped task actions and recovery",
        &[
            "unblock",
            "unblock-manual-fix",
            "recovery-summary",
            "recovery-integrated",
        ],
    ),
    ("17 Flow editor", &["flow-editor"]),
    (
        "18 Palette and existing picker variants",
        &[
            "palette",
            "palette-projects",
            "palette-sessions",
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
    ("22 New session", &["new-session", "new-session-composer"]),
];

const FIXTURES: &[Fixture] = &[
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
        view: "workbench",
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
        view: "new-session",
        state: "ordinary",
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
        "graph-ordinary" | "workbench-summary" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]},
                {"id":"t2","title":"Verify rendering","goal":"Check captured output","kind":"testing","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Capture is complete"}],"depends_on":["t1"]}
            ]}}));
            app.navigation.screen = Screen::Graph;
            app.graph_inspector_visible = true;
            if id == "workbench-summary" {
                app.navigation.screen = Screen::Workbench;
                app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
                app.details = Some(Details::new("worker:t1".into()));
            }
        }
        "graph-empty" | "graph-compact" => {
            app.navigation.screen = Screen::Graph;
            if id == "graph-compact" {
                app.notice = Some("Compact viewport · 40×12".into());
            }
        }
        "workbench-agent" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]}
            ]}}));
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
            app.details = Some(Details::new("worker:t1".into()));
        }
        "workbench-chat-empty" | "new-session-composer" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.focus = NavigationFocus::Composer;
            if id == "new-session-composer" {
                app.navigation.target = WorkbenchTarget::Session;
                let mut details = Details::new("orchestrator".into());
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
        | "workbench-tab-results"
        | "workbench-tab-checks"
        | "workbench-tab-comments"
        | "workbench-tab-diff" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]}
            ]}}));
            app.view.messages.push(MessageEntry {
                id: "capture-message".into(),
                sent_at: None,
                sender: "user".into(),
                target: "worker:t1".into(),
                text: "Please inspect the captured session state".into(),
                state: "sent".into(),
                kind: "message".into(),
            });
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
            let tab = match id {
                "workbench-tab-conversation" => WorkbenchTab::Conversation,
                "workbench-tab-results" => WorkbenchTab::Results,
                "workbench-tab-checks" => WorkbenchTab::Checks,
                "workbench-tab-comments" => WorkbenchTab::Comments,
                _ => WorkbenchTab::Diff,
            };
            app.navigation
                .update_presentation(|presentation| presentation.tab = tab);
            let mut details = Details::new("worker:t1".into());
            details.task = Some(
                json!({"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","status":"working","round":1,"change_kind":"code"}),
            );
            app.details = Some(details);
            if id == "workbench-tab-diff" {
                let mut diff = DiffView::new("t1".into());
                diff.load(Ok(json!({
                    "base":"0123456789abcdef",
                    "head":"fedcba9876543210",
                    "files":[{"path":"capture.rs","old_path":null,"status":"modified","added":1,"removed":1,"binary":false,
                        "lines":[{"kind":"hunk","old":1,"new":1,"text":"@@ -1 +1 @@"},
                            {"kind":"del","old":1,"new":null,"text":"old capture state"},
                            {"kind":"add","old":null,"new":1,"text":"verified capture state"}]}]
                })));
                app.diff = Some(diff);
            }
        }
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
            app.approval = app
                .view
                .approvals
                .first()
                .cloned()
                .map(approval::ApprovalCard::new);
        }
        "flow-editor" => {
            app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p-fixture","tasks":[
                {"id":"t1","title":"Prepare implementation","goal":"Implement the requested change","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior is verified"}],"depends_on":[]}
            ]}}));
            app.flow_editor = Some(FlowEditor::edit_task(&app.view, &app.view.tasks[0]));
        }
        "plan-view" => {
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
        "native-pty" | "native-pty-empty-history" | "native-pty-error" => {
            app.navigation.screen = Screen::Workbench;
            app.navigation.target = WorkbenchTarget::Session;
            app.navigation.focus = NavigationFocus::WorkbenchContent;
            app.navigation
                .update_presentation(|presentation| presentation.split = true);
            let mut details = Details::new("orchestrator".into());
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
            let cells = if id == "native-pty-empty-history" {
                Vec::new()
            } else {
                vec![
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
                ]
            };
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
        "config-error" => {
            let mut editor = config::ConfigEditor::new(
                include_str!("../../../examples/antislop.toml"),
                "fixture.toml".into(),
            )
            .expect("config fixture parses");
            if let Some(field) = editor.fields.first_mut() {
                field.error = Some("Fixture validation error".into());
            }
            app.config_editor = Some(editor);
        }
        "plan-history-loading" | "plan-history-empty" | "plan-history-error" => {
            let mut browser = planhistory::Browser::new();
            if id == "plan-history-empty" {
                browser.loading = false;
                browser.total = 0;
            }
            if id == "plan-history-error" {
                browser.loading = false;
                browser.error = Some("Fixture history read failed".into());
            }
            app.plan_history = Some(browser);
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
        "unblock" | "unblock-manual-fix" => {
            app.unblock = Some(unblock::Unblock::new(
                "p-fixture",
                &crate::lean::tui::view::TaskRow {
                    id: "t4".into(),
                    title: "Integrate reviewed change".into(),
                    status: "needs_user".into(),
                    reason: Some("Manual changes overlap with the reviewed worker branch".into()),
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

fn color(color: ratatui::style::Color) -> String {
    format!("{color:?}")
}

fn capture(fixture: Fixture, geometry: Geometry) -> Value {
    let Geometry {
        id: geometry_id,
        width,
        height,
    } = geometry;
    let mut app = App::test_app();
    apply_fixture(&mut app, fixture.id);
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
    let workbench_header = buffer
        .content
        .iter()
        .take(usize::from(width))
        .map(|cell| cell.symbol())
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
    let na_reason = match fixture.id {
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
        "new-session-composer" if height < 24 => {
            Some("Workbench compact layout replaces the session composer below 24 rows")
        }
        _ if width == 12 && height == 5 => Some(
            "12x5 is the minimum terminal guard viewport; full surface content and semantic styling are intentionally not applicable",
        ),
        _ => None,
    };
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
            line.contains("capture.rs") && line.contains("+1") && line.contains("−1")
        });
    let workbench_renderer_reached = app.navigation.screen == Screen::Workbench
        && app.details.is_some()
        && !app.workbench_areas.header.is_empty()
        && !app.workbench_areas.content.is_empty()
        && workbench_header.contains("Podsumowanie");
    let reached_view = match fixture.id {
        "graph-ordinary" | "graph-empty" | "graph-compact" => {
            app.navigation.screen == Screen::Graph
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
        | "workbench-tab-diff" => {
            app.navigation.screen == Screen::Workbench && agent_selected(&app, "worker:t1")
        }
        "decision-inbox" => app.decision_inbox.is_some(),
        "interview-question" | "interview-scope" => app.view.interview_question.is_some(),
        "cli-approval" => app.approval.is_some(),
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
        _ => false,
    };
    let reached_state = match fixture.id {
        "graph-empty" => app.view.tasks.is_empty(),
        "graph-compact" => width == 40 && height == 12 && !graph_inspector,
        "workbench-chat-empty" => app.input.text.is_empty() && app.navigation.focus == NavigationFocus::Composer,
        "decision-inbox" => app.decision_inbox.is_some(),
        "interview-question" => app.view.interview_question.as_ref().is_some_and(|(_, question)| question.id == "scope-confirmation") && !app.view.user_decision,
        "interview-scope" => app.view.user_decision && app.view.scope_confirmation_summary.is_some(),
        "cli-approval" => app.approval.as_ref().is_some_and(|card| card.approval.request == "approval-fixture-42" && card.approval.options.len() == 2),
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
        "native-pty-empty-history" => native_panel && native_fixture_state.1,
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
        _ => false,
    };
    json!({"schema":1,"fixture":fixture.id,"target":fixture.view,"view":fixture.view,"state":fixture.state,
        "geometry":geometry_id,
        "name":format!("{}.{}.{}.{}x{}",fixture.view,fixture.state,geometry_id,width,height),
        "width":width,"height":height,"cell_count":cells.len(),"cells":cells,
        "na_reason":na_reason,"native_panel":native_panel,
        "panels":{"workbench_list_visible":app.workbench_areas.list_visible,
            "workbench_split":app.workbench_areas.split,
            "workbench_content":{"x":app.workbench_areas.content.x,"y":app.workbench_areas.content.y,"width":app.workbench_areas.content.width,"height":app.workbench_areas.content.height},
            "graph_inspector_rendered":graph_inspector,
            "diff_file_list_rendered":diff_file_list_rendered},
        "active_tab":format!("{:?}",app.navigation.presentation().tab),
        "workbench_header":workbench_header,
        "reached_view":reached_view,"reached_state":reached_state,
        "decision_inbox":app.decision_inbox.is_some(),"flow_editor":app.flow_editor.is_some(),
        "unblock":app.unblock.is_some(),
        "task_integration":app.view.tasks.first().and_then(|task| task.integration.clone())})
}

fn text_has_inspector(cells: &[Value]) -> bool {
    cells
        .iter()
        .filter_map(|cell| cell["text"].as_str())
        .collect::<String>()
        .contains("INSPECTOR")
}

fn agent_selected(app: &App, key: &str) -> bool {
    matches!(&app.navigation.target, WorkbenchTarget::Agent(selected) if selected == key)
}

fn assert_capture_contract(record: &Value, fixture: Fixture, geometry: Geometry) {
    let Geometry { id, width, height } = geometry;
    assert_eq!(record["fixture"], fixture.id);
    assert_eq!(record["target"], fixture.view);
    assert_eq!(record["view"], fixture.view);
    assert_eq!(record["state"], fixture.state);
    assert_eq!(record["geometry"], id);
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
            record["workbench_header"]
                .as_str()
                .is_some_and(|header| header.contains("Podsumowanie")),
            "{} did not render the Workbench D3 header",
            fixture.id
        );
    }
    assert_eq!(
        record["cells"].as_array().unwrap().len(),
        usize::from(width) * usize::from(height)
    );
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
            let expected = width >= 100 && height >= 24;
            assert_eq!(
                panels["workbench_list_visible"], expected,
                "Workbench list boundary must use the actual post-draw layout"
            );
        }
        "split-below-height" | "split-at-height" | "split-above-height" => {
            let expected = height >= 24;
            assert_eq!(
                panels["workbench_split"], expected,
                "Workbench split boundary must use the actual post-draw layout"
            );
        }
        "diff-below-width" | "diff-at-width" | "diff-above-width" => {
            assert_eq!(
                panels["workbench_content"]["width"], width,
                "diff threshold sample records the real content panel width"
            );
            assert_eq!(
                panels["diff_file_list_rendered"],
                width >= 90,
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
        "workbench-tab-diff" => assert_eq!(record["active_tab"], "Diff"),
        "cli-approval" => assert!(
            text.contains("asks permission") || record["na_reason"].is_string(),
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
            assert!(
                record["native_panel"].as_bool().unwrap(),
                "empty-history native panel routing was not reached"
            );
            assert!(
                !text.contains("Please inspect"),
                "empty history must have a distinct terminal fixture"
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
        let mut expected = expected_surface_style(fixture, &theme);
        if expected.fg.is_none() && expected.bg.is_none() && expected.add_modifier.is_empty() {
            expected = theme.title();
        }
        let expected_fg = expected.fg.map(|color| format!("{color:?}"));
        let expected_bg = expected.bg.map(|color| format!("{color:?}"));
        let expected_modifiers = format!("{:?}", expected.add_modifier);
        let expected_style_present = visible.iter().any(|cell| {
            expected_fg.as_ref().is_none_or(|fg| cell["fg"] == *fg)
                && expected_bg.as_ref().is_none_or(|bg| cell["bg"] == *bg)
                && (expected.add_modifier.is_empty()
                    || expected_modifiers.split(" | ").all(|modifier| {
                        cell["modifiers"].as_str().is_some_and(|actual| {
                            actual.split(" | ").any(|actual| actual == modifier)
                        })
                    }))
        });
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

fn expected_surface_style(fixture: Fixture, theme: &Theme) -> ratatui::style::Style {
    match fixture.view {
        "decision-inbox" => theme.card(),
        "questions-scope" => theme.title(),
        "cli-consent" => ratatui::style::Style::default()
            .bg(theme.raised().bg.unwrap_or(ratatui::style::Color::Reset)),
        "composer" => theme.card(),
        "flow-editor" => ratatui::style::Style::default()
            .add_modifier(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::REVERSED),
        "filters" => ratatui::style::Style::default().fg(theme::ayu::BLUE),
        "new-session" => theme.dim(),
        "palette"
        | "picker-projects"
        | "picker-sessions"
        | "picker-agents"
        | "picker-recipients"
        | "picker-history"
        | "picker-slash-commands"
        | "rigor-picker"
        | "delete-confirmation" => {
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED)
        }
        "unblock" => theme.warning(),
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
    for (fixture, geometry) in selected_samples() {
        let record = capture(fixture, geometry);
        assert_capture_contract(&record, fixture, geometry);
    }
}

#[test]
#[ignore = "writes a new durable capture run; set ANTISLOP_CAPTURE_DIR to a new directory"]
fn export_catalog() {
    let directory = std::env::var_os("ANTISLOP_CAPTURE_DIR")
        .expect("set ANTISLOP_CAPTURE_DIR to a new run directory");
    let path = Path::new(&directory);
    std::fs::create_dir(path).expect("capture output directory must not already exist");
    let samples = selected_samples();
    let mut renders = Vec::with_capacity(samples.len());
    for (fixture, geometry) in samples {
        let record = capture(fixture, geometry);
        assert_capture_contract(&record, fixture, geometry);
        renders.push(record);
    }
    let mut file = std::fs::File::create(path.join("final-renders.json"))
        .expect("create final render catalog");
    serde_json::to_writer_pretty(&mut file, &json!({
        "format":"antislop-ratatui-testbackend-cells-v1",
        "source":"App::draw on ratatui::Terminal<TestBackend>",
        "states_note":"Fixture-level ordinary, empty, waiting, error, no_history, and compact states are separate real App fixtures where applicable. Each record includes na_reason for unavailable responsive states or surfaces. PTY panels use deterministic native terminal snapshots; no CLI provider starts.",
        "renders":renders,
    })).expect("serialize capture catalog");
    file.flush().expect("flush capture catalog");
    let manifest =
        std::fs::File::create(path.join("manifest.json")).expect("create capture manifest");
    serde_json::to_writer_pretty(
        manifest,
        &json!({
            "format":"antislop-ratatui-testbackend-cells-v1",
            "source":"App::draw on ratatui::Terminal<TestBackend>",
            "schema":1,
            "geometries":GEOMETRIES.iter().map(|g| json!({"id":g.id,"width":g.width,"height":g.height})).collect::<Vec<_>>(),
            "fixtures":FIXTURES.iter().map(|f| json!({"fixture":f.id,"view":f.view,"state":f.state})).collect::<Vec<_>>(),
            "section_2_catalog":SECTION_2_CATALOG.iter().map(|(view,fixtures)| json!({"view":view,"fixtures":fixtures})).collect::<Vec<_>>(),
            "geometry_applicability":[
                {"geometries":["inspector-below-width","inspector-at-width","inspector-above-width","inspector-below-height","inspector-above-height"],"fixtures":["graph-ordinary"],"not_applicable_reason":"The inspector threshold belongs to the Graph renderer; other targets have no graph inspector panel."},
                {"geometries":["list-below-width","list-at-width","list-above-width","list-below-height","list-above-height"],"fixtures":["workbench-summary"],"not_applicable_reason":"The 100x24 target is the Workbench agent list; other surfaces do not render that list."},
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
}
