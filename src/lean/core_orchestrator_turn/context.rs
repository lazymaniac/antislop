use super::super::*;
use super::prompt::{PromptContext, orchestrator_prompt_text};
use crate::lean::core_planning_context::planning_ledger_context;
use crate::lean::core_unanswered::includes_comment;

#[path = "context/comments.rs"]
mod comments;
use comments::comment_context;

pub(super) struct OrchestrationContext<'a> {
    conversation: &'a str,
    upto: u64,
    answered: u64,
    asked: Vec<store::Message>,
    is_comment: bool,
    config: Arc<Config>,
    project: PathBuf,
    open: Vec<Comment>,
    comment_text: String,
    workflow: store::Workflow,
    interviewing: bool,
    graph: Vec<Task>,
    editable_graph: bool,
    graph_json: String,
    history: Vec<store::Message>,
    ledger_context: Option<String>,
}

pub(super) fn prepare<'a>(
    core: &Core,
    conversation: &'a str,
    upto: u64,
) -> Result<OrchestrationContext<'a>> {
    let request = request_context(core, conversation, upto)?;
    let config = core.config_snapshot();
    let project = PathBuf::from(core.store.conversation(conversation)?.project);
    let (workflow, graph, graph_json) = plan_view(core, conversation)?;
    let (history, ledger_context) = history_context(core, conversation, request.answered)?;
    Ok(OrchestrationContext {
        conversation,
        upto,
        answered: request.answered,
        asked: request.asked,
        is_comment: request.is_comment,
        config,
        project,
        open: request.open,
        comment_text: request.comment_text,
        interviewing: workflow.interviewing(),
        workflow,
        editable_graph: !graph.is_empty(),
        graph,
        graph_json,
        history,
        ledger_context,
    })
}

struct RequestContext {
    answered: u64,
    asked: Vec<store::Message>,
    is_comment: bool,
    open: Vec<Comment>,
    comment_text: String,
}

fn request_context(core: &Core, conversation: &str, upto: u64) -> Result<RequestContext> {
    let answered = core.store.last_orchestration_answered(conversation)?;
    let asked = core.asked_since(conversation, answered, upto)?;
    let is_comment = includes_comment(&asked);
    let open = open_comments(core, conversation, answered)?;
    let comment_text = comment_context(core, conversation, upto, answered, is_comment)?;
    Ok(RequestContext {
        answered,
        asked,
        is_comment,
        open,
        comment_text,
    })
}

fn open_comments(core: &Core, conversation: &str, answered: u64) -> Result<Vec<Comment>> {
    Ok(core
        .store
        .comments(conversation, "open")?
        .into_iter()
        .filter(|comment| comment.message <= answered)
        .collect())
}

fn turn_setup<'a>(
    conversation: &'a str,
    upto: u64,
    project: &Path,
    config: &Config,
    is_comment: bool,
) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::Role,
        conversation,
        plan_id: None,
        role: if is_comment {
            COMMENT_ORCHESTRATOR
        } else {
            ORCHESTRATOR
        }
        .into(),
        profile: config.resolve(WorkKind::Orchestration, RoleKind::Orchestrator),
        cwd: project.to_path_buf(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        upto_message: upto,
    }
}

fn plan_view(core: &Core, conversation: &str) -> Result<(store::Workflow, Vec<Task>, String)> {
    let workflow = load_workflow(core, conversation)?;
    let (graph, graph_json) = load_graph(core, conversation)?;
    Ok((workflow, graph, graph_json))
}

fn load_workflow(core: &Core, conversation: &str) -> Result<store::Workflow> {
    let workflow = core.store.workflow(conversation)?;
    if workflow.planning() {
        announce_planning(core, conversation)?;
    }
    Ok(workflow)
}

fn load_graph(core: &Core, conversation: &str) -> Result<(Vec<Task>, String)> {
    let graph = core.store.latest_plan(conversation)?;
    record_revision_view(core, conversation, &graph)?;
    let encoded = graph_json(core, &graph)?;
    Ok((graph, encoded))
}

fn render_prompt(context: &OrchestrationContext<'_>, first: bool, fresh: bool) -> String {
    orchestrator_prompt_text(PromptContext {
        project: &context.project,
        first,
        fresh,
        answered: context.answered,
        asked: &context.asked,
        open: &context.open,
        history: &context.history,
        comment_context: &context.comment_text,
        editable_graph: context.editable_graph,
        graph: &context.graph,
        graph_json: &context.graph_json,
        interviewing: context.interviewing,
        workflow: &context.workflow,
        ledger_context: context.ledger_context.as_deref(),
    })
}

fn announce_planning(core: &Core, conversation: &str) -> Result<()> {
    core.store.append_event(
        conversation,
        None,
        None,
        "planning_progress",
        &json!({"text":"Working on the plan draft"}),
    )?;
    Ok(())
}

fn record_revision_view(core: &Core, conversation: &str, graph: &[Task]) -> Result<()> {
    let view = graph
        .first()
        .map(|task| {
            Ok::<_, Error>((
                task.plan_id.clone(),
                core.store.plan_revision(&task.plan_id)?,
            ))
        })
        .transpose()?;
    core.revision_views
        .lock()
        .expect("revision views mutex")
        .insert(conversation.into(), view);
    Ok(())
}

fn graph_json(core: &Core, graph: &[Task]) -> Result<String> {
    let base_revision = graph
        .first()
        .map(|task| core.store.plan_revision(&task.plan_id))
        .transpose()?;
    serde_json::to_string(&json!({"base_revision":base_revision,"tasks":graph.iter().map(|task|&task.spec).collect::<Vec<_>>()})).map_err(Error::from)
}

pub(super) fn ask(
    core: &Core,
    context: &OrchestrationContext<'_>,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    let setup = turn_setup(
        context.conversation,
        context.upto,
        &context.project,
        &context.config,
        context.is_comment,
    );
    core.turn(setup, cancel, |first, fresh| {
        render_prompt(context, first, fresh)
    })
}

#[path = "context/answer.rs"]
mod answer;
pub(super) fn finish(
    core: &Arc<Core>,
    context: &OrchestrationContext<'_>,
    answer: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    answer::finish(core, context, answer, cancel)
}

fn history_context(
    core: &Core,
    conversation: &str,
    answered: u64,
) -> Result<(Vec<store::Message>, Option<String>)> {
    let history = core.store.recent_messages_through(conversation, answered)?;
    let ledger = planning_ledger_context(core, conversation)?;
    Ok((history, ledger))
}
