//! Visibility probe. The pre-run rule and command outputs are recorded by the
//! runner in .antislop/probe-evidence, independently of fixture observations.
//! Deterministic choices are validated against actual writer input records.
mod common;
mod support;

use antislop::{config::Config, lean::Core};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=probe",
            "-c",
            "user.email=probe@example.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "probe did not become idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn options_visibility_for_writer_and_writing_gate() {
    for (case, unmerged, report) in [
        ("normal-opaque", false, "done"),
        ("unmerged-opaque", true, "done"),
        (
            "unmerged-accurate",
            true,
            r#"Mode contract: {"format":"json","field":"records"}"#,
        ),
        (
            "unmerged-wrong",
            true,
            r#"W1 evidence: mode.txt now uses v2 structured records; legacy parser is incompatible. Mode contract: {"format":"lines"}"#,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let project = project.canonicalize().unwrap();
        git(&project, &["init", "--quiet", "-b", "main"]);
        std::fs::write(project.join("mode.txt"), "7\n11\n").unwrap();
        git(&project, &["add", "."]);
        git(&project, &["commit", "--quiet", "-m", "initial"]);
        common::trust(&project);
        let codex = dir.path().join("codex");
        let agy = dir.path().join("agy");
        let source = r#"#!/usr/bin/env python3
import json, os, re, sys
prompt = sys.argv[-1]
if prompt == 'fixture W1':
    open('mode.txt', 'w').write('{"records":[7,11]}\n')
    sys.exit(0)
if os.path.basename(sys.argv[0]) == 'codex':
    plan = {'tasks': [
        {'id':'W1','title':'Change mode','goal':'Change mode','kind':'implementation','writes':True,'scope':['mode.txt'],'verifier':{'focus':'mode'}},
        {'id':'W2','title':'Use mode','goal':'Choose mode consumer','kind':'implementation','writes':True,'depends_on':['W1'],'scope':['mode.txt'],'verifier':{'focus':'mode'}},
        {'id':'gate','title':'Test mode','goal':'Choose mode checks','kind':'testing','writes':True,'depends_on':['W1'],'scope':['mode.txt'],'verifier':{'focus':'mode'}}]}
    answer = 'done' if '[runtime]' in prompt else '```plan\n'+json.dumps(plan)+'\n```'
    print(json.dumps({'type':'thread.started','thread_id':'planner'}))
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
    print(json.dumps({'type':'turn.completed','usage':{}}))
else:
    if 'You are a worker' in prompt and re.search(r'Task id: (W2-options|gate-options)\n', prompt):
        task = re.search(r'Task id: ([^\n]+)', prompt).group(1)
        version = open('mode.txt').read().strip()
        upstream = ''
        try:
            json.loads(version)
            choice={'format':'json','field':'records'}
        except ValueError:
            choice={'format':'lines'}
        observation = {'task':task,'cwd':os.getcwd(),'version':version,'choice':choice,'upstream':upstream,'prompt':prompt}
        with open(os.path.join(os.path.dirname(sys.argv[0]),'observations.jsonl'),'a') as log:
            log.write(json.dumps(observation)+'\n')
        answer = json.dumps({'version':version,'choice':choice,'upstream':upstream})+'\nSelected: '+choice['format']
    elif 'You are a worker' in prompt and 'Task id: W1-options' in prompt:
        answer='Compare focused mode change with a broader migration. Selected: A'
    elif 'You are a worker' in prompt:
        task = re.search(r'Task id: ([^\n]+)', prompt).group(1)
        with open(os.path.join(os.path.dirname(sys.argv[0]),'writers.jsonl'),'a') as log:
            log.write(json.dumps({'task':task,'version':open('mode.txt').read().strip(),'cwd':os.getcwd()})+'\n')
        if task == 'W1':
            open('mode.txt', 'w').write('{"records":[7,11]}\n')
            answer = open(os.path.join(os.path.dirname(sys.argv[0]),'w1-report')).read()
        else:
            answer = 'No changes needed.'
    else:
        if 'You verify' in prompt and 'Task: Change mode\n' in prompt:
            assert json.load(open('mode.txt')) == {'records':[7,11]}
        ids = re.findall(r'^- ([A-Za-z0-9_.-]+): ', prompt, re.M)
        answer = '```verdict\n'+json.dumps({'approved':True,'findings':[],'coverage':{i:{'status':'ok','note':'fixture inspected'} for i in ids}})+'\n```'
    print(json.dumps({'event':'init','conversation_id':'fixture'}))
    print(json.dumps({'event':'result','result':{'conversation_id':'fixture','status':'SUCCESS','response':answer,'usage':{}}}))
"#;
        std::fs::write(dir.path().join("w1-report"), report).unwrap();
        common::install_probe_cli(&codex, source);
        common::install_probe_cli(&agy, source);
        let text = include_str!("../examples/antislop.toml")
            .replace("binary = \"codex\"", &format!("binary = {:?}", codex))
            .replace("binary = \"agy\"", &format!("binary = {:?}", agy))
            .replace("required_gates = true", "required_gates = false")
            .replace("local_commits = true", "local_commits = false");
        let core = Core::open(
            &project.join(".antislop"),
            Config::from_toml(&text).unwrap(),
        )
        .unwrap();
        let conversation = support::fixture_conversation(&core, &project).unwrap();
        core.send(&conversation.id, "Probe dependent mode choices")
            .unwrap();
        idle(&core, &conversation.id);
        let tasks = core.store.latest_plan(&conversation.id).unwrap();
        assert!(!tasks.is_empty(), "planner produced no tasks");
        let plan = tasks[0].plan_id.clone();
        if unmerged {
            core.store.queue_plan(&conversation.id, &plan).unwrap();
            let base = git(&project, &["rev-parse", "HEAD"]);
            let worktree = project.join(".antislop/worktrees/probe-W1");
            git(
                &project,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    "probe-W1",
                    worktree.to_str().unwrap(),
                    &base,
                ],
            );
            // Execute W1's deterministic writer only inside its isolated checkout.
            assert!(
                Command::new(&agy)
                    .arg("fixture W1")
                    .current_dir(&worktree)
                    .status()
                    .unwrap()
                    .success()
            );
            git(&worktree, &["add", "mode.txt"]);
            git(&worktree, &["commit", "--quiet", "-m", "W1 result"]);
            let result = git(&worktree, &["rev-parse", "HEAD"]);
            for mut task in tasks {
                if task.spec["id"] == "W1" {
                    task.status = "approved".into();
                    task.round = 1;
                    task.base = Some(base.clone());
                    task.result = Some(result.clone());
                    task.report = Some(report.into());
                    core.store.update_task(&task).unwrap();
                } else if task.spec["id"] == "W1-options" {
                    task.status = "approved".into();
                    task.round = 1;
                    task.report = Some("Selected: structured".into());
                    core.store.update_task(&task).unwrap();
                }
            }
            core.resume_plan(&conversation.id, &plan).unwrap();
        } else {
            core.accept_plan(&conversation.id, &plan).unwrap();
        }
        idle(&core, &conversation.id);
        if !unmerged {
            let w1 = core
                .store
                .plan(&plan)
                .unwrap()
                .into_iter()
                .find(|t| t.spec["id"] == "W1")
                .unwrap();
            assert_eq!(w1.status, "approved", "{w1:?}");
            assert!(w1.round > 0 && w1.base.is_some() && w1.result.is_some());
            assert_eq!(
                git(
                    &project,
                    &["show", &format!("{}:mode.txt", w1.result.unwrap())]
                ),
                r#"{"records":[7,11]}"#
            );
        }
        let observations: Vec<Value> =
            std::fs::read_to_string(dir.path().join("observations.jsonl"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
        assert_eq!(observations.len(), 2, "{observations:?}");
        let writers: Vec<Value> = std::fs::read_to_string(dir.path().join("writers.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(writers.len(), if unmerged { 2 } else { 3 }, "{writers:?}");
        for id in ["W2", "gate"] {
            let writer = writers.iter().find(|item| item["task"] == id).unwrap();
            assert_eq!(writer["version"], r#"{"records":[7,11]}"#);
            assert_ne!(writer["cwd"], project.to_str().unwrap());
        }
        for id in ["W2-options", "gate-options"] {
            let observation = observations.iter().find(|item| item["task"] == id).unwrap();
            assert_ne!(observation["cwd"], project.to_str().unwrap());
            assert_eq!(observation["version"], r#"{"records":[7,11]}"#);
            assert!(!observation["prompt"].as_str().unwrap().contains(report));
            let task = core
                .store
                .plan(&plan)
                .unwrap()
                .into_iter()
                .find(|t| t.spec["id"] == id)
                .unwrap();
            assert_eq!(task.status, "approved", "{task:?}");
            assert!(task.base.is_none() && task.result.is_none());
            // Independent oracle: execute the selected decoding rule on the
            // actual bytes observed in the writer checkout, not report wording.
            let writer_id = id.strip_suffix("-options").unwrap();
            let writer = writers.iter().find(|w| w["task"] == writer_id).unwrap();
            let input = writer["version"].as_str().unwrap();
            let decoded: Option<Vec<i64>> = match observation["choice"]["format"].as_str() {
                Some("json") => serde_json::from_str::<Value>(input)
                    .ok()
                    .and_then(|v| v.get(observation["choice"]["field"].as_str()?).cloned())
                    .and_then(|v| serde_json::from_value(v).ok()),
                Some("lines") => input.lines().map(|line| line.parse::<i64>().ok()).collect(),
                _ => None,
            };
            let pass = decoded.as_deref() == Some(&[7, 11]);
            assert!(pass, "{case}: {observation}");
            println!(
                "probe {}",
                json!({"case":case,"task":id,"read":observation["version"],"report":report,"choice":observation["choice"],"writer_input":input,"decoded":decoded,"outcome":if pass {"PASS"} else {"FAIL"}})
            );
        }
        assert_eq!(
            std::fs::read_to_string(project.join("mode.txt")).unwrap(),
            if unmerged {
                "7\n11\n"
            } else {
                "{\"records\":[7,11]}\n"
            }
        );
    }
}
