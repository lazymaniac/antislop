//! Protocol-6 operation declarations, in wire order (including error messages).
//! Regular operations share fields across the grouped API and flat serde model.
//! Start, Events and Recover alone need explicit application adapters.

use super::request::SessionActionKind;

// This local grammar handles unit/record operations, field serde attributes,
// and the three adapters below. It generates whole enums and exhaustive matches:
// no fall-through dispatch, boxed probing or runtime registry is involved.
macro_rules! operations {
    ($( $mode:ident $group:ident($request:ident) $variant:ident
        $( { $( $(#[$attr:meta])* $field:ident: $ty:ty, )* } )? , )*) => {
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
        pub(crate) enum WireRequest {
            $( $variant $( { $( $(#[$attr])* $field: $ty, )* } )?, )*
        }
        operations!(@collect
            [
                [Start, StartIn { project: String },]
                [Recover(crate::lean::TaskRecoveryRequest),]
                [
                    Events { conversation: String, after: u64, wait_ms: u64 },
                    EventsPaged { conversation: String, after: u64, wait_ms: u64, limit: Option<usize> },
                ]
                []
                []
            ]
            [
                Request::Control(ControlRequest::Start) => WireRequest::Start { project: None },
                Request::Control(ControlRequest::StartIn { project }) => WireRequest::Start { project: Some(project) },
                Request::History(HistoryRequest::Events { conversation, after, wait_ms }) =>
                    WireRequest::Events { conversation, after, wait_ms, limit: None },
                Request::History(HistoryRequest::EventsPaged { conversation, after, wait_ms, limit }) =>
                    WireRequest::Events { conversation, after, wait_ms, limit },
                Request::Conversation(ConversationRequest::Recover(crate::lean::TaskRecoveryRequest {
                    conversation, plan, task, action, consent_files, freshness_token, message
                })) => WireRequest::Recover { conversation, plan, task, action, consent_files, freshness_token, message },
            ]
            [
                WireRequest::Start { project } => Request::Control(match project {
                    Some(project) => ControlRequest::StartIn { project },
                    None => ControlRequest::Start,
                }),
                WireRequest::Events { conversation, after, wait_ms, limit } => Request::History(match limit {
                    Some(_) => HistoryRequest::EventsPaged { conversation, after, wait_ms, limit },
                    None => HistoryRequest::Events { conversation, after, wait_ms },
                }),
                WireRequest::Recover { conversation, plan, task, action, consent_files, freshness_token, message } =>
                    Request::Conversation(ConversationRequest::Recover(crate::lean::TaskRecoveryRequest {
                        conversation, plan, task, action, consent_files, freshness_token, message
                    })),
            ];
            $( $mode $group($request) $variant $( { $( $field: $ty, )* } )?, )*
        );
    };
    // Adapter wire fields were already emitted; application variants are above.
    (@collect $groups:tt $encode:tt $decode:tt;
        adapter $group:ident($request:ident) $variant:ident $fields:tt, $($rest:tt)*) => {
        operations!(@collect $groups $encode $decode; $($rest)*);
    };
    (@collect $groups:tt [$($encode:tt)*] [$($decode:tt)*];
        regular $group:ident($request:ident) $variant:ident
        $( { $( $field:ident: $ty:ty, )* } )?, $($rest:tt)*) => {
        operations!(@place $group
            [$variant $( { $( $field: $ty, )* } )?,]
            $groups
            [
                $($encode)*
                Request::$group($request::$variant $( { $( $field, )* } )?) =>
                    WireRequest::$variant $( { $( $field, )* } )?,
            ]
            [
                $($decode)*
                WireRequest::$variant $( { $( $field, )* } )? =>
                    Request::$group($request::$variant $( { $( $field, )* } )?),
            ];
            $($rest)*
        );
    };
    (@collect [$control:tt $conversation:tt $history:tt $session:tt $terminal:tt]
        $encode:tt $decode:tt;) => {
        operations!(@finish [
            Control(ControlRequest)$control
            Conversation(ConversationRequest)$conversation
            History(HistoryRequest)$history
            Session(SessionRequest)$session
            Terminal(TerminalRequest)$terminal
        ] $encode $decode);
    };
    (@finish [$( $group:ident($request:ident)[$($variants:tt)*] )*]
        [$($encode:tt)*] [$($decode:tt)*]) => {
        #[derive(Debug, Clone, PartialEq)]
        pub enum Request { $( $group($request), )* }
        $(
            #[derive(Debug, Clone, PartialEq)]
            pub enum $request { $($variants)* }
        )*
        pub(super) fn encode(request: Request) -> WireRequest {
            match request { $($encode)* }
        }
        pub(super) fn decode(wire: WireRequest) -> Request {
            match wire { $($decode)* }
        }
    };

    // Fixed buckets: Control, Conversation, History, Session, Terminal.
    // Collecting by group keeps the declaration in its original wire order,
    // so serde's expected-variant list also stays compatible.
    (@place Control [$($variant:tt)*] [[$($members:tt)*] $conversation:tt $history:tt $session:tt $terminal:tt]
        $encode:tt $decode:tt; $($rest:tt)*) => {
        operations!(@collect [[$($members)* $($variant)*] $conversation $history $session $terminal]
            $encode $decode; $($rest)*);
    };
    (@place Conversation [$($variant:tt)*] [$control:tt [$($members:tt)*] $history:tt $session:tt $terminal:tt]
        $encode:tt $decode:tt; $($rest:tt)*) => {
        operations!(@collect [$control [$($members)* $($variant)*] $history $session $terminal]
            $encode $decode; $($rest)*);
    };
    (@place History [$($variant:tt)*] [$control:tt $conversation:tt [$($members:tt)*] $session:tt $terminal:tt]
        $encode:tt $decode:tt; $($rest:tt)*) => {
        operations!(@collect [$control $conversation [$($members)* $($variant)*] $session $terminal]
            $encode $decode; $($rest)*);
    };
    (@place Session [$($variant:tt)*] [$control:tt $conversation:tt $history:tt [$($members:tt)*] $terminal:tt]
        $encode:tt $decode:tt; $($rest:tt)*) => {
        operations!(@collect [$control $conversation $history [$($members)* $($variant)*] $terminal]
            $encode $decode; $($rest)*);
    };
    (@place Terminal [$($variant:tt)*] [$control:tt $conversation:tt $history:tt $session:tt [$($members:tt)*]]
        $encode:tt $decode:tt; $($rest:tt)*) => {
        operations!(@collect [$control $conversation $history $session [$($members)* $($variant)*]]
            $encode $decode; $($rest)*);
    };
}

operations! {
    regular Conversation(ConversationRequest) AddSource {
        conversation: String,
        text: String,
    },
    regular Conversation(ConversationRequest) GetSource {
        conversation: String,
        source_id: String,
    },
    regular Control(ControlRequest) ActivateRuntime {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        executable: String,
        path: Option<String>,
        conversation: Option<String>,
        task: Option<String>,
    },
    regular Control(ControlRequest) RuntimeActivationStatus,
    regular Control(ControlRequest) RuntimeActivationOperation {
        request_id: String,
    },
    regular Control(ControlRequest) CancelRuntimeActivation {
        request_id: String,
    },
    // Trusted-client operational information, never an execution grant.
    regular Control(ControlRequest) ClassifyRuntimeFailure {
        conversation: String,
        message: u64,
        reason: String,
    },
    regular Control(ControlRequest) RuntimeNotice {
        conversation: String,
        text: String,
    },
    regular Terminal(TerminalRequest) TerminalOpen {
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        #[serde(default)]
        retry: bool,
    },
    regular Terminal(TerminalRequest) TerminalRead {
        conversation: String,
        role: String,
        handle: String,
        after: u64,
    },
    regular Terminal(TerminalRequest) TerminalInput {
        conversation: String,
        role: String,
        handle: String,
        sequence: u64,
        input: crate::lean::terminal::Input,
    },
    regular Terminal(TerminalRequest) TerminalResize {
        conversation: String,
        role: String,
        handle: String,
        columns: u16,
        rows: u16,
    },
    regular Terminal(TerminalRequest) TerminalDetach {
        conversation: String,
        role: String,
        handle: String,
    },
    regular Session(SessionRequest) RunContext {
        conversation: String,
        turn: String,
    },
    regular Session(SessionRequest) SessionAction {
        conversation: String,
        action: SessionActionKind,
    },
    regular Conversation(ConversationRequest) Comment {
        conversation: String,
        operation_id: Option<String>,
        plan: Option<String>,
        target: String,
        text: String,
        round: Option<u32>,
        run: Option<String>,
    },
    regular Conversation(ConversationRequest) SendToAgent {
        message: crate::lean::store::AgentMessage,
    },
    regular History(HistoryRequest) TaskRounds {
        conversation: String,
        plan: String,
        id: String,
    },
    regular History(HistoryRequest) TaskRound {
        conversation: String,
        plan: String,
        id: String,
        round: u32,
        #[serde(default)]
        references: Option<bool>,
    },
    regular Session(SessionRequest) SessionEvents {
        conversation: String,
        turn: String,
        after: u64,
        limit: Option<usize>,
        #[serde(default)]
        include_messages: Option<bool>,
    },
    regular History(HistoryRequest) HistoryContent {
        conversation: String,
        owner: crate::lean::store::ContentOwner,
        offset: u64,
    },
    regular History(HistoryRequest) HistoryMessages {
        conversation: String,
        scope: crate::lean::store::HistoryScope,
        after: u64,
        limit: Option<usize>,
    },
    regular Session(SessionRequest) SessionContent {
        conversation: String,
        event: u64,
        field: String,
        offset: u64,
    },
    regular Session(SessionRequest) ResultDocuments {
        conversation: String,
    },
    regular Session(SessionRequest) ResultDocumentContent {
        conversation: String,
        id: String,
        offset: u64,
    },
    // Which binary the daemon runs and whether it is executing work.
    regular Control(ControlRequest) Identity,
    regular Control(ControlRequest) GetConfig,
    regular Control(ControlRequest) SetConfig {
        toml: String,
        expected_revision: String,
    },
    // Stops an idle daemon so a frontend from another build can replace it.
    regular Control(ControlRequest) Shutdown,
    regular Control(ControlRequest) ForceShutdown,
    regular Control(ControlRequest) Conversations,
    adapter Control(ControlRequest) Start {
        #[serde(default)]
        project: Option<String>,
    },
    regular Conversation(ConversationRequest) Send {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        text: String,
        #[serde(default)]
        to: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
    },
    // Rejects a proposed plan and asks the orchestrator to revise it.
    regular Conversation(ConversationRequest) Reject {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        plan: String,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        revision_token: Option<String>,
    },
    // Runs again what the plan left unfinished, through the same sessions.
    regular Conversation(ConversationRequest) Resume {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        plan: String,
        #[serde(default)]
        revision_token: Option<String>,
    },
    // The user finished commenting on the plan: the orchestrator answers
    // the draft comments together.
    regular Conversation(ConversationRequest) ReviewComments {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        plan: String,
    },
    adapter Conversation(ConversationRequest) Recover {
        conversation: String,
        plan: String,
        task: String,
        action: crate::lean::RecoveryAction,
        #[serde(default)]
        consent_files: Option<Vec<String>>,
        freshness_token: String,
        #[serde(default)]
        message: Option<String>,
    },
    // This frontend steers the conversation from now on; others watch.
    regular Conversation(ConversationRequest) TakeControl {
        conversation: String,
    },
    regular Conversation(ConversationRequest) RestartOrchestrator {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
    },
    regular Conversation(ConversationRequest) Cancel {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        #[serde(default)]
        revision_token: Option<String>,
    },
    // Stops starting new plan work, or continues it.
    regular Conversation(ConversationRequest) Pause {
        conversation: String,
        paused: bool,
    },
    // Sets who decides the approval dialogs of the conversation's agents:
    // `supervised` (the user) or `autonomous` (the orchestrator).
    regular Conversation(ConversationRequest) SetApprovalMode {
        conversation: String,
        mode: String,
    },
    // The user's decision on an agent CLI's approval dialog.
    regular Conversation(ConversationRequest) AnswerApproval {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        request: String,
        choice: String,
    },
    // Paginated read of all stored plans for a conversation.
    regular Conversation(ConversationRequest) Plans {
        conversation: String,
        #[serde(default)]
        offset: Option<usize>,
        #[serde(default)]
        limit: Option<usize>,
    },
    // Detailed history of a single plan.
    regular Conversation(ConversationRequest) Plan {
        conversation: String,
        plan: String,
    },
    // Comments of a conversation, optionally filtered by plan and/or status.
    regular Conversation(ConversationRequest) Comments {
        conversation: String,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    // The user's comment on one element of the team panel or a diff.
    // The user's comment scoped to a specific plan.
    // Resolves, reopens (`open`) or deletes a comment.
    regular Conversation(ConversationRequest) CommentStatus {
        conversation: String,
        id: u64,
        status: String,
    },
    // A writer's change as files and numbered lines.
    regular Conversation(ConversationRequest) Diff {
        conversation: String,
        plan: String,
        id: String,
    },
    // Reviews a writer's change now; findings go to the orchestrator.
    regular Conversation(ConversationRequest) Review {
        conversation: String,
        plan: String,
        id: String,
        #[serde(default)]
        revision_token: Option<String>,
    },
    // One task of a plan with its latest report and verdict.
    // Inspector snapshot and checks without a Git freshness fingerprint.
    regular Conversation(ConversationRequest) TaskChecks {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    regular Conversation(ConversationRequest) Task {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    // User-initiated plan revision (Fast-Path skips AI review unless requested).
    regular Conversation(ConversationRequest) RevisePlan {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        revision: Box<crate::lean::store::PlanRevision>,
        #[serde(default)]
        request_ai_review: bool,
    },
    // Selects the team template (rigor level) for a conversation.
    regular Conversation(ConversationRequest) SelectTeam {
        conversation: String,
        team: String,
        #[serde(default)]
        reason: String,
    },
    adapter History(HistoryRequest) Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    regular Session(SessionRequest) Sessions {
        conversation: String,
    },
    regular Session(SessionRequest) DeleteSession {
        conversation: String,
    },
    regular History(HistoryRequest) History {
        conversation: String,
        #[serde(default)]
        after: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    regular History(HistoryRequest) Mail {
        conversation: String,
    },
}
