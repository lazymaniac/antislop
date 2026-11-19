use super::*;

#[derive(Debug)]
pub(super) enum Route {
    Handled(Value),
    Unmatched(Request),
}

pub(super) fn handle(
    core: &Arc<Core>,
    project: &Path,
    config: &Path,
    request: Request,
    frontend: Option<(&str, &super::lease::Leases)>,
) -> Result<Value> {
    ensure_accepting(core, &request)?;
    let scope = super::lease::scope(&request);
    let observer = match (frontend, scope) {
        (Some((client, leases)), Some((conversation, access))) => {
            leases.access(conversation, client, access)?
        }
        _ => false,
    };
    let follows = matches!(scope, Some((_, super::lease::Access::Follow)));
    let mut reply = handle_request(core, project, config, request)?;
    if follows && let Some(reply) = reply.as_object_mut() {
        reply.insert("observer".into(), json!(observer));
    }
    Ok(reply)
}

fn handle_request(
    core: &Arc<Core>,
    project: &Path,
    config: &Path,
    request: Request,
) -> Result<Value> {
    let context = Context {
        core,
        project,
        config,
    };
    let operation = match &request {
        Request::Conversation(request) => request
            .operation()
            .map(|(conversation, id, kind)| (conversation.to_owned(), id.to_owned(), kind)),
        _ => None,
    };
    match operation {
        Some((conversation, id, kind)) => core.store.idempotent(&conversation, &id, kind, || {
            route_request(&context, request)
        }),
        None => route_request(&context, request),
    }
}

fn route_request(context: &Context<'_>, request: Request) -> Result<Value> {
    match route_all(context, request)? {
        Route::Handled(value) => Ok(value),
        Route::Unmatched(_) => Err(Error::Invalid("unhandled daemon request".into())),
    }
}

fn ensure_accepting(core: &Core, request: &Request) -> Result<()> {
    use super::{ControlRequest as C, ConversationRequest as Q, SessionRequest as S};
    let supervision = matches!(
        request,
        Request::Conversation(
            Q::Cancel { .. }
                | Q::Pause { .. }
                | Q::AnswerApproval { .. }
                | Q::SetApprovalMode { .. }
                | Q::TakeControl { .. }
        ) | Request::Terminal(super::TerminalRequest::TerminalInput { .. })
    );
    let changes_work = !supervision
        && matches!(
            super::lease::scope(request),
            Some((_, super::lease::Access::Change | super::lease::Access::Take))
        )
        || matches!(
            request,
            Request::Control(C::Start | C::StartIn { .. } | C::SetConfig { .. })
                | Request::Session(S::DeleteSession { .. })
        );
    require(
        !super::activation::pending(core) || !changes_work,
        "runtime activation is pending; current turns can finish, retry new work after readiness is confirmed",
    )?;
    require(
        !core.state.join("stopped").exists()
            || matches!(
                request,
                Request::Control(crate::lean::daemon::ControlRequest::Identity)
                    | Request::Control(crate::lean::daemon::ControlRequest::Shutdown)
                    | Request::Control(crate::lean::daemon::ControlRequest::ForceShutdown)
            ),
        "the daemon is stopping",
    )
}

type Router = for<'a> fn(&Context<'a>, Request) -> Result<Route>;

fn route_all(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            config::route,
            control::route,
            planning::route,
            conversation::route,
            terminal::route,
            history::route,
        ],
    )
}

fn route_pipeline(context: &Context<'_>, request: Request, routers: &[Router]) -> Result<Route> {
    let mut request = request;
    for router in routers {
        match router(context, request)? {
            Route::Handled(value) => return Ok(Route::Handled(value)),
            Route::Unmatched(unmatched) => request = unmatched,
        }
    }
    Ok(Route::Unmatched(request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_send_rejects_incompatible_orchestrator_before_message_acceptance() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let source = directory.path().join("attachment.png");
        std::fs::write(&source, crate::lean::content::valid_png_fixture()).unwrap();
        let config_text = include_str!("../../../../examples/antislop.toml").replace(
            "[profiles.orchestrator]\nprovider = \"codex\"",
            "[profiles.orchestrator]\nprovider = \"agy\"",
        );
        let config = Config::from_toml(&config_text).unwrap();
        let core = Arc::new(Core::open(&directory.path().join("state"), config).unwrap());
        let conversation = core
            .store
            .create_conversation(&project.to_string_lossy())
            .unwrap();

        let error = handle(
            &core,
            &project,
            &directory.path().join("config.toml"),
            Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
                conversation: conversation.id.clone(),
                operation_id: Some(crate::id()),
                text: "keep this message".into(),
                to: None,
                images: vec![source.to_string_lossy().into_owned()],
            }),
            None,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("image attachments are not supported"),
            "unexpected image send error: {error}"
        );
        assert!(error.to_string().contains("agy provider"));
        assert!(
            core.store
                .messages_after(&conversation.id, 0)
                .unwrap()
                .is_empty()
        );
        assert!(
            core.store
                .linked_images(&conversation.id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn image_send_uses_running_turn_provider_after_config_changes() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let source = directory.path().join("attachment.png");
        std::fs::write(&source, crate::lean::content::valid_png_fixture()).unwrap();
        let unsupported = include_str!("../../../../examples/antislop.toml").replace(
            "[profiles.orchestrator]\nprovider = \"codex\"",
            "[profiles.orchestrator]\nprovider = \"agy\"",
        );
        let core = Arc::new(
            Core::open(
                &directory.path().join("state"),
                Config::from_toml(&unsupported).unwrap(),
            )
            .unwrap(),
        );
        let config_path = directory.path().join("config.toml");
        std::fs::write(&config_path, unsupported).unwrap();
        let conversation = core
            .store
            .create_conversation(&project.to_string_lossy())
            .unwrap();
        core.store
            .start_turn(&conversation.id, "orchestrator", "agy", 0, None)
            .unwrap();

        let supported = include_str!("../../../../examples/antislop.toml").to_owned();
        let expected_revision = crate::hash(std::fs::read(&config_path).unwrap());
        handle(
            &core,
            &project,
            &config_path,
            Request::Control(crate::lean::daemon::ControlRequest::SetConfig {
                toml: supported,
                expected_revision,
            }),
            None,
        )
        .unwrap();
        let error = handle(
            &core,
            &project,
            &config_path,
            Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
                conversation: conversation.id.clone(),
                operation_id: Some(crate::id()),
                text: "keep this while the old turn runs".into(),
                to: None,
                images: vec![source.to_string_lossy().into_owned()],
            }),
            None,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("image attachments are not supported")
        );
        assert!(error.to_string().contains("agy provider"));
        assert!(
            core.store
                .messages_after(&conversation.id, 0)
                .unwrap()
                .is_empty()
        );
        assert!(
            core.store
                .linked_images(&conversation.id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn supported_image_send_and_text_only_send_still_accept() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let source = directory.path().join("attachment.png");
        std::fs::write(&source, crate::lean::content::valid_png_fixture()).unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Arc::new(Core::open(&directory.path().join("state"), config).unwrap());
        let conversation = core
            .store
            .create_conversation(&project.to_string_lossy())
            .unwrap();
        let endpoint = directory.path().join("config.toml");

        handle(
            &core,
            &project,
            &endpoint,
            Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
                conversation: conversation.id.clone(),
                operation_id: Some(crate::id()),
                text: "inspect this image".into(),
                to: None,
                images: vec![source.to_string_lossy().into_owned()],
            }),
            None,
        )
        .unwrap();
        assert_eq!(core.store.linked_images(&conversation.id).unwrap().len(), 1);

        handle(
            &core,
            &project,
            &endpoint,
            Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
                conversation: conversation.id.clone(),
                operation_id: Some(crate::id()),
                text: "text only".into(),
                to: None,
                images: vec![],
            }),
            None,
        )
        .unwrap();
        let messages = core.store.messages_after(&conversation.id, 0).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().any(|message| message.text == "text only"));
    }

    #[test]
    fn result_document_requests_list_and_read_persisted_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Arc::new(Core::open(&directory.path().join("state"), config).unwrap());
        let conversation = core
            .store
            .create_conversation(&project.to_string_lossy())
            .unwrap();
        let task = core
            .store
            .add_plan(
                &conversation.id,
                &[json!({
                    "id":"writer",
                    "title":"Write result",
                    "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Create the session result",
                    "kind":"writing",
                    "change_kind":"artifact",
                    "outputs":["brief.md"],
                    "reviewer":{"summary":"Check result"}
                })],
            )
            .unwrap()
            .remove(0);
        let text = "Persisted result: café 👩‍💻";
        core.store
            .record_result_document(&conversation.id, &task.id, "brief.md", "commit-1", text)
            .unwrap();

        let listed = handle(
            &core,
            &project,
            &directory.path().join("config.toml"),
            Request::Session(crate::lean::daemon::SessionRequest::ResultDocuments {
                conversation: conversation.id.clone(),
            }),
            None,
        )
        .unwrap();
        let documents = listed.as_array().unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0]["path"], "brief.md");
        assert_eq!(documents[0]["source_commit"], "commit-1");
        let id = documents[0]["id"].as_str().unwrap();

        let content = handle(
            &core,
            &project,
            &directory.path().join("config.toml"),
            Request::Session(crate::lean::daemon::SessionRequest::ResultDocumentContent {
                conversation: conversation.id,
                id: id.to_owned(),
                offset: 0,
            }),
            None,
        )
        .unwrap();
        assert_eq!(content["text"], text);
        assert_eq!(content["done"], true);
    }
}

pub(super) struct Context<'a> {
    pub(super) core: &'a Arc<Core>,
    pub(super) project: &'a Path,
    pub(super) config: &'a Path,
}

mod config;
mod control;
mod conversation;
mod history;
mod planning;
mod terminal;

fn validate_project_conversation(core: &Core, project: &Path, conversation: &str) -> Result<()> {
    let conv = core.store.conversation(conversation)?;
    let owner = core.store.repository_id(&project.to_string_lossy())?;
    let target = core.store.repository_id(&conv.project)?;
    require(
        conv.project == project.to_string_lossy() || (owner.is_some() && owner == target),
        &format!(
            "conversation {conversation} does not belong to project {}",
            project.display()
        ),
    )?;
    Ok(())
}

fn validate_conversation_plan(core: &Core, conversation: &str, plan: &str) -> Result<()> {
    require(
        core.store
            .plan_belongs_to_conversation(conversation, plan)?,
        &format!("plan {plan} does not belong to conversation {conversation}"),
    )?;
    Ok(())
}

fn validate_plan_task(core: &Core, plan: &str, task_id: &str) -> Result<()> {
    require(
        core.store.task_in_plan(plan, task_id)?,
        &format!("task {task_id} does not belong to plan {plan}"),
    )?;
    Ok(())
}
