use super::*;

fn plan(output: &str, delivery: Option<&str>) -> Value {
    json!({"tasks":[{"id":"report","title":"Analysis report","goal":"Deliver the report only","kind":"analysis","writes":true,"change_kind":"artifact","outputs":[output],"delivery":delivery}]})
}

#[test]
fn explicit_ignored_reports_are_valid_without_code_gates() {
    for output in [
        ".antislop/continuation-plan-review.md",
        ".antislop/reports/analysis.md",
        ".reports/analysis.md",
        "report.md",
    ] {
        let mut tasks = submitted(plan(output, Some("untracked"))).unwrap();
        for _ in 0..3 {
            tasks = with_options(with_required_agents(tasks));
            assert_eq!(tasks.len(), 1);
            validate(&tasks).unwrap();
        }
        assert_eq!(tasks[0].outputs, [output]);
        let worker = worker_turn_prompt("project", &tasks[0], None, "");
        let reviewer = verifier_prompt("project", &tasks[0], "Report", None);
        assert!(
            worker.contains("Explicit artifact outputs declared in your contract are retained")
        );
        assert!(reviewer.contains(output));
        assert!(reviewer.contains("inspect those documents"));
    }
}

#[test]
fn artifact_paths_still_reject_traversal_metadata_and_runtime_internals() {
    for output in [
        "/tmp/report.md",
        "../report.md",
        ".antislop/../report.md",
        "./.antislop/report.md",
        ".antislop//report.md",
        ".git/report.md",
        ".GIT/report.md",
        "nested/.git/report.md",
        ".antislop/worktrees/report.md",
        ".antislop/sessions/report.md",
        ".antislop/content/report.md",
        ".ANTISLOP/WORKTREES/report.md",
        ".antislop/*.md",
        "src/report.md",
        ".antislop/config.toml",
    ] {
        assert!(
            submitted(plan(output, Some("untracked"))).is_err(),
            "{output}"
        );
    }
    let error = submitted(plan(".antislop/report.md", None))
        .unwrap_err()
        .to_string();
    assert!(error.contains("delivery=untracked"), "{error}");
}
