use architect::config::*;
use std::collections::BTreeSet;

fn source() -> &'static str {
    include_str!("../examples/architect.toml")
}
fn config() -> Config {
    Config::from_toml(source()).unwrap()
}

#[test]
fn review_reply_pagination_defaults_without_changing_legacy_pins_or_permissions() {
    let original = config();
    assert_eq!(
        original.orchestrator.review_checks_per_part,
        default_review_checks()
    );
    let pinned = original.pin().unwrap();
    let stored = serde_json::to_string(&pinned).unwrap();
    assert!(!stored.contains("review_checks_per_part"));
    let restored: PinnedConfig = serde_json::from_str(&stored).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, pinned);
    let mut legacy = original.clone();
    legacy.orchestrator.review_checks_per_part = 0;
    let legacy = legacy.pin().unwrap();
    assert_ne!(legacy.content_hash, pinned.content_hash);
    assert_eq!(legacy.config.policy, original.policy);
    assert_eq!(legacy.config.routing, original.routing);
    assert!(
        serde_json::to_string(&legacy)
            .unwrap()
            .contains("\"review_checks_per_part\":0")
    );
}

#[test]
fn example_resolves_every_task_family_and_dedicated_review() {
    let config = config();
    let kinds = BTreeSet::from([
        WorkKind::Context,
        WorkKind::Requirements,
        WorkKind::Options,
        WorkKind::Planning,
        WorkKind::Architecture,
        WorkKind::Dependencies,
        WorkKind::Implementation,
        WorkKind::Refactoring,
        WorkKind::StaticAnalysis,
        WorkKind::TestDesign,
        WorkKind::Testing,
        WorkKind::Security,
        WorkKind::Performance,
        WorkKind::Integration,
        WorkKind::Research,
        WorkKind::Writing,
        WorkKind::Analysis,
        WorkKind::Delivery,
    ]);
    config.validate_work_kinds(&kinds).unwrap();
    for kind in kinds {
        let executor = config.resolve(kind, RoleKind::Executor).unwrap();
        let verifier = config.resolve(kind, RoleKind::Verifier).unwrap();
        assert!(!executor.requires_independent_session);
        assert!(verifier.requires_independent_session);
        assert_eq!(verifier.provider, ProviderKind::Claude);
    }
    let orchestrator = config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    assert_eq!(orchestrator.profile, "orchestrator");
    assert_eq!(orchestrator.provider, ProviderKind::Codex);
    assert_eq!(orchestrator.effort, Effort::Deep);
    assert_eq!(orchestrator.native_effort.as_deref(), Some("high"));
    assert_eq!(config.workspace.mode, WorkspaceMode::Worktree);
}

#[test]
fn load_reads_only_the_selected_file() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("selected.toml");
    std::fs::write(&path, source()).unwrap();
    assert_eq!(Config::load(&path).unwrap(), config());
    assert!(Config::load(&temporary.path().join("missing.toml")).is_err());
}

#[test]
fn no_implicit_orchestrator_or_provider_selection() {
    let absent = source().replace("\n[orchestrator]\nprofile = \"orchestrator\"\n", "\n");
    assert!(Config::from_toml(&absent).is_err());
    let mut invalid = config();
    invalid.orchestrator.profile = "".into();
    assert!(invalid.validate().is_err());
    invalid.orchestrator.profile = "missing".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn unknown_fields_and_secret_values_are_rejected_without_echo() {
    for input in [
        format!("unknown = 123\n{}", source()),
        source().replace(
            "binary = \"codex\"",
            "binary = \"codex\"\napi_key = \"DO_NOT_ECHO_SECRET\"",
        ),
        source().replace(
            "mode = \"supervised\"",
            "mode = \"supervised\"\npush = true",
        ),
        source().replace(
            "roles = [\"orchestrator\"]",
            "roles = [\"orchestrator\"]\naccess_token = \"DO_NOT_ECHO_SECRET\"",
        ),
    ] {
        let error = Config::from_toml(&input).unwrap_err().to_string();
        assert!(!error.contains("DO_NOT_ECHO_SECRET"));
    }
}

#[test]
fn duplicate_keys_and_invalid_enum_values_are_rejected() {
    for input in [
        source().replace(
            "schema_version = 1",
            "schema_version = 1\nschema_version = 1",
        ),
        source().replace("provider = \"codex\"", "provider = \"unknown\""),
        source().replace("effort = \"deep\"", "effort = \"maximum\""),
        source().replace("mode = \"supervised\"", "mode = \"magic\""),
        source().replace("mode = \"worktree\"", "mode = \"unknown\""),
        source().replace("schema_version = 1", "schema_version = 2"),
        source().replace("revision = 1", "revision = 0"),
    ] {
        assert!(Config::from_toml(&input).is_err());
    }
}

#[test]
fn missing_route_blocks_team_without_provider_fallback() {
    let mut config = config();
    config.routing.remove(&WorkKind::Research);
    assert!(config.validate().is_ok());
    assert!(
        config
            .resolve(WorkKind::Research, RoleKind::Executor)
            .is_err()
    );
    assert!(
        config
            .validate_work_kinds(&BTreeSet::from([WorkKind::Research]))
            .is_err()
    );
    assert!(
        config
            .resolve(WorkKind::Implementation, RoleKind::Executor)
            .is_ok()
    );
}

#[test]
fn role_and_work_restrictions_are_enforced_before_start() {
    let mut config = config();
    config
        .routing
        .get_mut(&WorkKind::Implementation)
        .unwrap()
        .verifier = "execution".into();
    assert!(config.validate().is_err());
    let mut config = self::config();
    config
        .profiles
        .get_mut("execution")
        .unwrap()
        .work_kinds
        .remove(&WorkKind::Implementation);
    assert!(config.validate().is_err());
    let mut config = self::config();
    config
        .profiles
        .get_mut("orchestrator")
        .unwrap()
        .roles
        .clear();
    assert!(config.validate().is_err());
}

#[test]
fn missing_provider_and_missing_native_effort_mapping_block_validation() {
    let mut config = config();
    config.providers.remove(&ProviderKind::Codex);
    assert!(config.validate().is_err());
    let mut config = self::config();
    config
        .providers
        .get_mut(&ProviderKind::Codex)
        .unwrap()
        .effort_map
        .remove(&Effort::Deep);
    assert!(config.validate().is_err());
    let mut config = self::config();
    config
        .providers
        .get_mut(&ProviderKind::Gemini)
        .unwrap()
        .effort_map
        .insert(Effort::Standard, "medium".into());
    assert!(config.validate().is_err());
}

#[test]
fn unsupported_native_effort_is_explicit_and_probe_can_reject_declared_support() {
    let config = config();
    let executor = config
        .resolve(WorkKind::Implementation, RoleKind::Executor)
        .unwrap();
    assert_eq!(executor.provider, ProviderKind::Gemini);
    assert_eq!(executor.effort, Effort::Standard);
    assert!(executor.native_effort.is_none());
    executor.validate_native_effort(&BTreeSet::new()).unwrap();
    let orchestrator = config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    assert!(
        orchestrator
            .validate_native_effort(&BTreeSet::new())
            .is_err()
    );
    orchestrator
        .validate_native_effort(&BTreeSet::from(["high".into()]))
        .unwrap();
}

#[test]
fn verifier_may_use_the_same_provider_but_requires_an_independent_session() {
    let mut config = config();
    let profile = config.profiles.get_mut("execution").unwrap();
    profile.roles.insert(RoleKind::Verifier);
    config
        .routing
        .get_mut(&WorkKind::Implementation)
        .unwrap()
        .verifier = "execution".into();
    let executor = config
        .resolve(WorkKind::Implementation, RoleKind::Executor)
        .unwrap();
    let verifier = config
        .resolve(WorkKind::Implementation, RoleKind::Verifier)
        .unwrap();
    assert_eq!(executor.provider, verifier.provider);
    assert!(verifier.requires_independent_session);
}

#[test]
fn pinned_run_assignments_survive_global_config_changes_and_detect_tampering() {
    let mut global = config();
    let pinned = global.pin().unwrap();
    global.profiles.get_mut("execution").unwrap().model = Some("new-global-model".into());
    global.policy.mode = Mode::Autonomous;
    global.policy.revision = 2;
    let old = pinned
        .config
        .resolve(WorkKind::Implementation, RoleKind::Executor)
        .unwrap();
    assert!(old.model.is_none());
    assert_eq!(pinned.config.policy.mode, Mode::Supervised);
    assert_ne!(pinned.content_hash, global.pin().unwrap().content_hash);
    let encoded = serde_json::to_vec(&pinned).unwrap();
    let restored: PinnedConfig = serde_json::from_slice(&encoded).unwrap();
    restored.validate().unwrap();
    let mut changed = restored;
    changed.config.policy.mode = Mode::Autonomous;
    assert!(changed.validate().is_err());
}

#[test]
fn supervised_requires_current_approval_for_stages_corrections_and_results() {
    let policy = config().policy;
    assert!(!policy.automatic_progression());
    for action in [
        LocalAction::Read,
        LocalAction::Write,
        LocalAction::Check,
        LocalAction::Commit,
        LocalAction::StartStage,
        LocalAction::StartCorrectionRound,
        LocalAction::AcceptStageResult,
    ] {
        assert!(policy.authorize_local(action, false).is_err());
        policy.authorize_local(action, true).unwrap();
    }
}

#[test]
fn autonomous_local_progression_respects_restricted_local_capabilities() {
    let mut policy = config().policy;
    policy.mode = Mode::Autonomous;
    assert!(policy.automatic_progression());
    for action in [
        LocalAction::Write,
        LocalAction::Check,
        LocalAction::Commit,
        LocalAction::StartStage,
        LocalAction::StartCorrectionRound,
        LocalAction::AcceptStageResult,
    ] {
        policy.authorize_local(action, false).unwrap();
    }
    policy.local_write = false;
    policy.local_checks = false;
    policy.local_commits = false;
    for action in [LocalAction::Write, LocalAction::Check, LocalAction::Commit] {
        assert!(policy.authorize_local(action, true).is_err());
    }
    policy.authorize_local(LocalAction::Read, false).unwrap();
}

fn grant(policy: &Policy, action: ExternalAction) -> ExternalGrant {
    ExternalGrant {
        id: "grant-1".into(),
        actor: "authenticated-user".into(),
        action,
        scope_hash: "exact-action-scope".into(),
        policy_revision: policy.revision,
        approved: true,
    }
}

#[test]
fn neither_mode_implicitly_allows_external_or_destructive_actions() {
    for mode in [Mode::Supervised, Mode::Autonomous] {
        let mut policy = config().policy;
        policy.mode = mode;
        for action in [
            ExternalAction::Push,
            ExternalAction::Publish,
            ExternalAction::ExternalMessage,
            ExternalAction::Destructive,
        ] {
            assert!(
                policy
                    .authorize_external(action, "exact-action-scope", None)
                    .is_err()
            );
            let grant = grant(&policy, action);
            policy
                .authorize_external(action, "exact-action-scope", Some(&grant))
                .unwrap();
        }
    }
}

#[test]
fn grant_must_match_action_scope_and_policy_and_be_an_attributed_approval() {
    let policy = config().policy;
    let valid = grant(&policy, ExternalAction::Push);
    let mut invalid = vec![];
    let mut value = valid.clone();
    value.action = ExternalAction::Publish;
    invalid.push(value);
    let mut value = valid.clone();
    value.scope_hash = "older-scope".into();
    invalid.push(value);
    let mut value = valid.clone();
    value.policy_revision += 1;
    invalid.push(value);
    let mut value = valid.clone();
    value.approved = false;
    invalid.push(value);
    let mut value = valid.clone();
    value.actor = "  ".into();
    invalid.push(value);
    let mut value = valid.clone();
    value.id.clear();
    invalid.push(value);
    for grant in invalid {
        assert!(
            policy
                .authorize_external(ExternalAction::Push, "exact-action-scope", Some(&grant))
                .is_err()
        );
    }
    assert!(
        policy
            .authorize_external(ExternalAction::Push, "", Some(&valid))
            .is_err()
    );
}

#[test]
fn invalid_binary_model_and_managed_home_are_rejected() {
    let mut config = config();
    config
        .providers
        .get_mut(&ProviderKind::Codex)
        .unwrap()
        .binary = "".into();
    assert!(config.validate().is_err());
    let mut config = self::config();
    config.profiles.get_mut("orchestrator").unwrap().model = Some(" \n ".into());
    assert!(config.validate().is_err());
    let mut config = self::config();
    config
        .providers
        .get_mut(&ProviderKind::Gemini)
        .unwrap()
        .managed_home = Some("relative-home".into());
    assert!(config.validate().is_err());
}

#[test]
fn orchestration_verifier_route_is_explicit_and_executor_matches_orchestrator() {
    let mut configured = config();
    let review = configured
        .resolve(WorkKind::Orchestration, RoleKind::Verifier)
        .unwrap();
    assert!(review.requires_independent_session);
    assert_eq!(review.profile, "review");
    assert!(
        configured
            .resolve(WorkKind::Orchestration, RoleKind::Executor)
            .is_err()
    );
    configured
        .routing
        .get_mut(&WorkKind::Orchestration)
        .unwrap()
        .executor = "execution".into();
    assert!(configured.validate().is_err());
    let mut configured = config();
    configured.routing.remove(&WorkKind::Orchestration);
    assert!(configured.validate_work_kinds(&BTreeSet::new()).is_err());
}

#[test]
fn explicit_role_provider_limits_keep_codex_in_orchestration_and_agy_in_task_roles() {
    let mut config = config();
    for profile in config.profiles.values_mut() {
        if !profile.roles.contains(&RoleKind::Orchestrator) {
            profile.provider = ProviderKind::Agy;
        }
    }
    config.role_providers = [
        (
            RoleKind::Orchestrator,
            BTreeSet::from([ProviderKind::Codex]),
        ),
        (RoleKind::Executor, BTreeSet::from([ProviderKind::Agy])),
        (RoleKind::Verifier, BTreeSet::from([ProviderKind::Agy])),
    ]
    .into();
    config.validate().unwrap();
    for kind in config.routing.keys() {
        let role = if *kind == WorkKind::Orchestration {
            RoleKind::Orchestrator
        } else {
            RoleKind::Executor
        };
        let expected = if role == RoleKind::Orchestrator {
            ProviderKind::Codex
        } else {
            ProviderKind::Agy
        };
        assert_eq!(config.resolve(*kind, role).unwrap().provider, expected);
        assert_eq!(
            config.resolve(*kind, RoleKind::Verifier).unwrap().provider,
            ProviderKind::Agy
        );
    }
    let mut invalid = config.clone();
    invalid.profiles.get_mut("reasoning").unwrap().provider = ProviderKind::Codex;
    assert!(
        invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("role provider limit")
    );
    let mut invalid = config.clone();
    invalid.profiles.get_mut("review").unwrap().provider = ProviderKind::Claude;
    assert!(invalid.validate().is_err());
    let mut invalid = config;
    invalid
        .role_providers
        .insert(RoleKind::Executor, BTreeSet::new());
    assert!(invalid.validate().is_err());
}
