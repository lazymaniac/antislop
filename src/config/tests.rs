use super::*;
use crate::Error;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[test]
fn configuration_errors_explain_location_and_repair_without_values() {
    let example = include_str!("../../examples/antislop.toml");
    for (source, expected, repair) in [
        (
            example.replace("effort = \"high\"", "effort = \"secret-value\""),
            "unsupported value",
            "allowed values",
        ),
        (
            example.replace("[policy]", "[policy]\nunknown_setting = \"secret-value\""),
            "unknown field `unknown_setting`",
            "remove the unknown field",
        ),
        (
            example.replace("local_commits = true", "local_commits = \"secret-value\""),
            "incorrect value or type",
            "expected a boolean",
        ),
    ] {
        let error = Config::from_toml(&source).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        assert!(error.contains(repair), "{error}");
        assert!(!error.contains("secret-value"), "{error}");
    }
    let error = Config::from_toml("schema_version = [")
        .unwrap_err()
        .to_string();
    assert!(error.contains("line 1, column"), "{error}");
    assert!(error.contains("to fix:"), "{error}");
}

#[test]
fn effort_levels_reach_the_cli() {
    let example = include_str!("../../examples/antislop.toml");
    for (name, expected) in [
        ("low", "low"),
        ("medium", "medium"),
        ("high", "high"),
        ("xhigh", "xhigh"),
    ] {
        let config = Config::from_toml(
            &example.replace("effort = \"high\"", &format!("effort = \"{name}\"")),
        )
        .unwrap();
        for provider in [
            ProviderKind::Codex,
            ProviderKind::Agy,
            ProviderKind::Claude,
            ProviderKind::Opencode,
            ProviderKind::Gemini,
        ] {
            let mut config = config.clone();
            let provider_config = config.providers[&ProviderKind::Codex].clone();
            config.providers.insert(provider, provider_config);
            config.profiles.get_mut("orchestrator").unwrap().provider = provider;
            let resolved = config
                .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
                .unwrap();
            assert_eq!(
                resolved.native_effort.as_deref(),
                (provider != ProviderKind::Gemini).then_some(expected)
            );
            assert_eq!(serde_json::to_value(resolved.effort).unwrap(), expected);
        }
    }
    assert!(Config::from_toml(&example.replace("effort = \"high\"", "effort = \"typo\"")).is_err());
}

#[test]
fn the_example_routes_every_kind_to_a_worker_and_a_reviewer() {
    let text = include_str!("../../examples/antislop.toml");
    let config = Config::from_toml(text).unwrap();
    for kind in config
        .routing
        .keys()
        .filter(|kind| **kind != WorkKind::Orchestration)
    {
        assert_eq!(
            config.resolve(*kind, RoleKind::Executor).unwrap().work_kind,
            *kind
        );
        let reviewer = config.resolve(*kind, RoleKind::Reviewer).unwrap();
        assert_eq!(reviewer.role, RoleKind::Reviewer);
    }
    let orchestrator = config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    assert_eq!(orchestrator.provider, ProviderKind::Codex);
    assert_eq!(orchestrator.native_effort.as_deref(), Some("high"));
}

#[test]
fn profile_command_supports_platform_launchers_for_workers_and_reviewers() {
    let text = include_str!("../../examples/antislop.toml")
        .replace(
            "[profiles.execution]\nprovider = \"agy\"",
            "[profiles.execution]\nprovider = \"codex\"\ncommand = \"omlx launch codex\"",
        )
        .replace(
            "[profiles.review]\nprovider = \"agy\"",
            "[profiles.review]\nprovider = \"codex\"\ncommand = [\"ollama\", \"launch\", \"codex\"]",
        );
    let config = Config::from_toml(&text).unwrap();
    let orchestrator = config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    assert_eq!(orchestrator.binary, PathBuf::from("codex"));
    assert!(orchestrator.args.is_empty());

    let worker = config
        .resolve(WorkKind::Implementation, RoleKind::Executor)
        .unwrap();
    assert_eq!(worker.provider, ProviderKind::Codex);
    assert_eq!(worker.binary, PathBuf::from("omlx"));
    assert_eq!(worker.args, vec!["launch", "codex"]);
    assert_eq!(worker.native_effort.as_deref(), Some("high"));

    let reviewer = config
        .resolve(WorkKind::Implementation, RoleKind::Reviewer)
        .unwrap();
    assert_eq!(reviewer.provider, ProviderKind::Codex);
    assert_eq!(reviewer.binary, PathBuf::from("ollama"));
    assert_eq!(reviewer.args, vec!["launch", "codex"]);
    assert_eq!(reviewer.native_effort.as_deref(), Some("medium"));
}

#[test]
fn command_resolution_preserves_precedence_and_argument_order() {
    let provider = ProviderConfig {
        binary: PathBuf::from("provider"),
        args: vec!["provider-arg".into()],
        command: None,
    };
    let mut profile = Profile {
        provider: ProviderKind::Codex,
        binary: None,
        args: vec!["profile-arg".into()],
        command: None,
        model: None,
        effort: Effort::High,
    };
    assert_eq!(
        resolve_command(&provider, &profile).unwrap(),
        (
            PathBuf::from("provider"),
            vec!["provider-arg".into(), "profile-arg".into()]
        )
    );

    let provider = ProviderConfig {
        command: Some(CommandLine(vec!["provider-wrapper".into(), "base".into()])),
        ..provider
    };
    assert_eq!(
        resolve_command(&provider, &profile).unwrap(),
        (
            PathBuf::from("provider-wrapper"),
            vec!["base".into(), "provider-arg".into(), "profile-arg".into()]
        )
    );

    profile.binary = Some(PathBuf::from("profile-binary"));
    assert_eq!(
        resolve_command(&provider, &profile).unwrap(),
        (PathBuf::from("profile-binary"), vec!["profile-arg".into()])
    );

    profile.command = Some(CommandLine(vec![
        "profile-wrapper".into(),
        "command".into(),
    ]));
    assert_eq!(
        resolve_command(&provider, &profile).unwrap(),
        (
            PathBuf::from("profile-wrapper"),
            vec!["command".into(), "profile-arg".into()]
        )
    );
}

#[test]
fn a_route_to_a_missing_profile_is_rejected() {
    let mut config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    config
        .routing
        .get_mut(&WorkKind::Requirements)
        .unwrap()
        .executor = "nobody".into();
    assert!(config.validate().is_err());
}

#[test]
fn routing_assigns_a_shared_profile_to_different_work_kinds_and_roles() {
    let mut config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    let shared = config.profiles["execution"].clone();
    config.profiles = BTreeMap::from([("shared".into(), shared)]);
    config.orchestrator.profile = "shared".into();
    for route in config.routing.values_mut() {
        route.executor = "shared".into();
        route.reviewer = "shared".into();
    }
    config.validate().unwrap();
    for kind in config.routing.keys().copied() {
        let worker_role = if kind == WorkKind::Orchestration {
            RoleKind::Orchestrator
        } else {
            RoleKind::Executor
        };
        for role in [worker_role, RoleKind::Reviewer] {
            let resolved = config.resolve(kind, role).unwrap();
            assert_eq!(resolved.profile, "shared");
            assert_eq!(resolved.work_kind, kind);
            assert_eq!(resolved.role, role);
        }
    }
    // Profile reuse must not introduce implicit routes or invalid roles.
    assert!(
        config
            .resolve(WorkKind::Implementation, RoleKind::Orchestrator)
            .is_err()
    );
    assert!(
        config
            .resolve(WorkKind::Orchestration, RoleKind::Executor)
            .is_err()
    );
    config.routing.remove(&WorkKind::Context);
    assert!(
        config
            .resolve(WorkKind::Context, RoleKind::Executor)
            .is_err()
    );
    assert!(
        config
            .resolve(WorkKind::Context, RoleKind::Reviewer)
            .is_err()
    );
}

#[test]
fn assignments_still_require_existing_profiles_and_consistent_orchestration() {
    let original = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    let mut config = original.clone();
    config.orchestrator.profile = "missing".into();
    assert!(config.validate().is_err());

    let mut config = original.clone();
    config
        .routing
        .get_mut(&WorkKind::Requirements)
        .unwrap()
        .reviewer = "missing".into();
    assert!(config.validate().is_err());

    let mut config = original;
    config
        .routing
        .get_mut(&WorkKind::Orchestration)
        .unwrap()
        .executor = "execution".into();
    let error = config.validate().unwrap_err().to_string();
    assert!(
        error.contains("routing.orchestration.executor must match orchestrator.profile"),
        "{error}"
    );
}

#[test]
fn obsolete_configuration_is_rejected_without_echoing_values() {
    let example = include_str!("../../examples/antislop.toml");
    for (header, field) in [
        ("[providers.codex]", "isolation = \"read_only\""),
        ("[providers.codex]", "isolation_levels = [\"read_only\"]"),
        ("[providers.codex]", "managed_home = \"/secret/home\""),
        ("[providers.codex]", "effort_map = { high = \"secret\" }"),
        ("[profiles.orchestrator]", "native_effort = \"secret\""),
        (
            "[profiles.orchestrator]",
            "work_kinds = [\"orchestration\"]",
        ),
        ("[profiles.orchestrator]", "roles = [\"orchestrator\"]"),
        ("[policy]", "revision = 1"),
        ("[policy]", "local_checks = true"),
        ("[orchestrator]", "review_checks_per_part = 24"),
        ("", "role_providers = { executor = [\"agy\"] }"),
    ] {
        let source = if header.is_empty() {
            format!("{field}\n{example}")
        } else {
            example.replace(header, &format!("{header}\n{field}"))
        };
        let error = Config::from_toml(&source).unwrap_err().to_string();
        assert!(
            error.contains("invalid TOML configuration"),
            "{field}: {error}"
        );
        assert!(!error.contains("secret"), "{error}");
    }
    for effort in ["light", "standard", "deep"] {
        assert!(
            Config::from_toml(
                &example.replace("effort = \"high\"", &format!("effort = \"{effort}\""))
            )
            .is_err()
        );
    }
    assert!(Config::from_toml(&format!("{example}\n[workspace]\nmode = \"in-place\"\n")).is_err());
}

#[test]
fn required_rigor_routes_are_always_validated() {
    let source = include_str!("../../examples/antislop.toml");
    for kind in [
        WorkKind::Options,
        WorkKind::Research,
        WorkKind::Architecture,
        WorkKind::TestDesign,
        WorkKind::Delivery,
        WorkKind::Testing,
    ] {
        let mut config = Config::from_toml(source).unwrap();
        config.routing.remove(&kind);
        assert!(config.validate().is_err(), "accepted missing {kind:?}");
    }
}

#[test]
fn the_planner_resolves_the_planning_executor_and_nothing_else() {
    let mut config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    config
        .routing
        .get_mut(&WorkKind::Planning)
        .unwrap()
        .executor = "execution".into();
    let planner = config
        .resolve(WorkKind::Planning, RoleKind::Planner)
        .unwrap();
    assert_eq!(planner.profile, "execution");
    assert_eq!(planner.role, RoleKind::Planner);
    for kind in [WorkKind::Orchestration, WorkKind::Implementation] {
        assert!(config.resolve(kind, RoleKind::Planner).is_err());
    }
    config.routing.remove(&WorkKind::Planning);
    assert!(config.validate().is_err());
    assert!(
        config
            .resolve(WorkKind::Planning, RoleKind::Planner)
            .is_err()
    );
}

#[test]
fn retired_policy_fields_are_rejected_with_migration_instructions() {
    let source = include_str!("../../examples/antislop.toml");
    for field in ["required_gates", "local_options"] {
        let text = source.replace("[policy]", &format!("[policy]\n{field} = false"));
        let error = Config::from_toml(&text).unwrap_err().to_string();
        assert!(
            error.contains("remove policy.required_gates") && error.contains("schema_version = 5"),
            "{error}"
        );
    }
    let error = Config::from_toml(&source.replace("schema_version = 5", "schema_version = 4"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("schema_version = 5"), "{error}");
}

#[test]
fn load_and_resolve_preserve_file_error_context_and_role_selection() {
    let fixture = crate::test_resources::tempdir();
    let path = fixture.path().join("antislop.toml");
    std::fs::write(&path, include_str!("../../examples/antislop.toml")).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(
        loaded
            .resolve(WorkKind::Implementation, RoleKind::Executor)
            .unwrap()
            .profile,
        "execution"
    );
    assert_ne!(
        loaded
            .resolve(WorkKind::Implementation, RoleKind::Reviewer)
            .unwrap()
            .profile,
        "execution"
    );

    let invalid = fixture.path().join("invalid.toml");
    std::fs::write(&invalid, "schema_version = [").unwrap();
    let error = Config::load(&invalid).unwrap_err().to_string();
    assert!(error.contains(invalid.to_str().unwrap()), "{error}");
    assert!(error.contains("line 1, column"), "{error}");
    assert!(error.contains("to fix:"), "{error}");

    let error = Config::load(&fixture.path().join("missing-config-test.toml"));
    assert!(matches!(error, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound));
}

#[test]
fn previous_configuration_schemas_are_rejected_without_a_fallback() {
    for version in [2, 3] {
        let source = include_str!("../../examples/antislop.toml")
            .replace("schema_version = 5", &format!("schema_version = {version}"));
        let error = Config::from_toml(&source).unwrap_err().to_string();
        assert!(error.contains("schema_version = 5"), "{error}");
    }
}

#[test]
fn removed_permission_and_execution_budget_settings_are_rejected() {
    let source = include_str!("../../examples/antislop.toml");
    for setting in [
        r#"mode = "supervised""#,
        "unrestricted = true",
        "local_write = true",
        "max_parallel = 4",
        "max_tasks = 100",
    ] {
        let candidate = source.replace("[policy]", &format!("[policy]\n{setting}"));
        assert!(
            Config::from_toml(&candidate)
                .unwrap_err()
                .to_string()
                .contains("unknown field"),
            "{setting}"
        );
    }
}

#[test]
fn legacy_context_route_is_removed_before_validation_and_serialization() {
    let text = format!(
        "{}\n[routing.context]\nexecutor = \"deleted-context-agent\"\nreviewer = \"deleted-reviewer\"\n",
        include_str!("../../examples/antislop.toml")
    );
    let config = Config::from_toml(&text).unwrap();
    assert!(!config.routing.contains_key(&WorkKind::Context));
    assert!(
        serde_json::to_value(&config).unwrap()["routing"]
            .get("context")
            .is_none()
    );
    assert!(
        config
            .resolve(WorkKind::Requirements, RoleKind::Executor)
            .is_ok()
    );
}
