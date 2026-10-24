use super::*;
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
            example.replace("max_parallel = 4", "max_parallel = 0"),
            "policy.max_parallel",
            "positive integer",
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
fn the_example_routes_every_kind_to_a_worker_and_a_verifier() {
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
        let verifier = config.resolve(*kind, RoleKind::Verifier).unwrap();
        assert_eq!(verifier.role, RoleKind::Verifier);
    }
    let orchestrator = config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    assert_eq!(orchestrator.provider, ProviderKind::Codex);
    assert_eq!(orchestrator.native_effort.as_deref(), Some("high"));
}

#[test]
fn profile_command_supports_platform_launchers_for_workers_and_verifiers() {
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

    let verifier = config
        .resolve(WorkKind::Implementation, RoleKind::Verifier)
        .unwrap();
    assert_eq!(verifier.provider, ProviderKind::Codex);
    assert_eq!(verifier.binary, PathBuf::from("ollama"));
    assert_eq!(verifier.args, vec!["launch", "codex"]);
    assert_eq!(verifier.native_effort.as_deref(), Some("medium"));
}

#[test]
fn a_route_to_a_missing_profile_is_rejected() {
    let mut config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    config.routing.get_mut(&WorkKind::Context).unwrap().executor = "nobody".into();
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
        route.verifier = "shared".into();
    }
    config.validate().unwrap();
    for kind in config.routing.keys().copied() {
        let worker_role = if kind == WorkKind::Orchestration {
            RoleKind::Orchestrator
        } else {
            RoleKind::Executor
        };
        for role in [worker_role, RoleKind::Verifier] {
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
            .resolve(WorkKind::Context, RoleKind::Verifier)
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
    config.routing.get_mut(&WorkKind::Context).unwrap().verifier = "missing".into();
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
fn max_parallel_defaults_to_four_and_must_be_positive() {
    let text = include_str!("../../examples/antislop.toml");
    let default = Config::from_toml(&text.replace("max_parallel = 4\n", "")).unwrap();
    assert_eq!(default.policy.max_parallel, 4);
    assert!(Config::from_toml(&text.replace("max_parallel = 4", "max_parallel = 0")).is_err());
    let one = Config::from_toml(&text.replace("max_parallel = 4", "max_parallel = 1")).unwrap();
    assert_eq!(one.policy.max_parallel, 1);
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
    assert!(
        Config::from_toml(&example.replace("mode = \"task-worktrees\"", "mode = \"worktree\""))
            .is_err()
    );
}

#[test]
fn options_route_is_required_even_without_required_gates() {
    let text = include_str!("../../examples/antislop.toml");
    for required_gates in [true, false] {
        let source = text.replace(
            "required_gates = true",
            &format!("required_gates = {required_gates}"),
        );
        let mut config = Config::from_toml(&source).unwrap();
        let route = config.routing.remove(&WorkKind::Options).unwrap();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("routing.options is required"), "{error}");
        config.routing.insert(WorkKind::Options, route);
        assert!(config.validate().is_ok());
    }
}

#[test]
fn required_gates_need_routes_for_the_work_they_add() {
    let text = include_str!("../../examples/antislop.toml");
    assert!(Config::from_toml(text).is_ok());
    // Find the first route table of a gate kind and take it away.
    let header = text
        .lines()
        .find(|line| line.starts_with("[routing.") && line.contains("static_analysis"))
        .expect("the example routes static analysis");
    let start = text.find(header).unwrap();
    let end = text[start + header.len()..]
        .find("\n[")
        .map_or(text.len(), |at| start + header.len() + at);
    let without = format!("{}{}", &text[..start], &text[end..]);
    let error = Config::from_toml(&without).unwrap_err().to_string();
    assert!(error.contains("required_gates"), "{error}");
    // With the gates off, the route is not needed.
    let off = without.replace("required_gates = true", "required_gates = false");
    assert!(Config::from_toml(&off).is_ok());
}
