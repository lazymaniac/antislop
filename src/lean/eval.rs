//! `antislop eval`: calibration of verifiers on seeded cases (design M5).
//!
//! Each case is a small repository with a worker's change and report, and
//! what a careful verifier should conclude. Every case runs in two arms on a
//! real CLI: `verifier`, the prompt and obligations a task's verifier gets
//! (with the runtime's one follow-up for unaccounted obligations), and
//! `baseline`, one agent asked plainly to review the same change. Results go
//! to stdout as JSON lines; a summary goes to stderr.
//!
//! A case directory holds `case.json`, `base/` (the project before) and
//! optionally `change/` (files written over it, the worker's change):
//!
//! ```json
//! {"scenario": 3, "task": {TaskSpec}, "report": "...",
//!  "expect": {"approve": false, "mentions": ["assert", "tautolog"]}}
//! ```
//!
//! A rejection counts as finding the seeded defect when a finding names one
//! of `mentions` (case-insensitive).

use super::agent::{self, Access, Spec};
use super::tasks::{self, TaskSpec, Verdict};
use crate::config::ProviderKind;
use crate::{Error, Result, require};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    scenario: u32,
    task: TaskSpec,
    report: String,
    expect: Expect,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    approve: bool,
    #[serde(default)]
    mentions: Vec<String>,
    /// Several seeded defects, each named by any of its words: a catch
    /// needs every one of them.
    #[serde(default)]
    defects: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Arm {
    Verifier,
    Baseline,
}

impl Arm {
    fn name(self) -> &'static str {
        match self {
            Arm::Verifier => "verifier",
            Arm::Baseline => "baseline",
        }
    }
}

struct Options {
    suite: PathBuf,
    provider: ProviderKind,
    binary: PathBuf,
    model: Option<String>,
    effort: Option<String>,
    arms: Vec<Arm>,
    only: Vec<String>,
    jobs: usize,
}

struct Grading {
    suite: PathBuf,
    files: Vec<PathBuf>,
}

const USAGE: &str = "usage: antislop eval SUITE --cli codex|claude|agy|gemini|opencode \
     [--binary PATH] [--model M] [--effort E] [--arm verifier|baseline] \
     [--case NAME]... [--jobs N]\n       antislop eval SUITE --grade RESULTS.jsonl...";

/// `antislop eval SUITE --grade RESULTS.jsonl...`
fn grading(args: &[String]) -> Result<Option<Grading>> {
    let Some(at) = args.iter().position(|arg| arg == "--grade") else {
        return Ok(None);
    };
    let suite = args
        .iter()
        .take(at)
        .find(|arg| !arg.starts_with('-'))
        .ok_or_else(|| Error::Invalid(USAGE.into()))?;
    let files: Vec<PathBuf> = args[at + 1..].iter().map(PathBuf::from).collect();
    require(!files.is_empty(), "--grade needs result files")?;
    Ok(Some(Grading {
        suite: PathBuf::from(suite),
        files,
    }))
}

fn parse(args: &[String]) -> Result<Options> {
    let mut suite = None;
    let mut provider = None;
    let mut binary = None;
    let mut model = None;
    let mut effort = None;
    let mut arms = vec![Arm::Verifier, Arm::Baseline];
    let mut only = vec![];
    let mut jobs = 2;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| Error::Invalid(format!("{arg} needs a value\n{USAGE}")))
        };
        match arg.as_str() {
            "--cli" => {
                provider = Some(match value()?.as_str() {
                    "codex" => ProviderKind::Codex,
                    "claude" => ProviderKind::Claude,
                    "agy" => ProviderKind::Agy,
                    "gemini" => ProviderKind::Gemini,
                    "opencode" => ProviderKind::Opencode,
                    other => return Err(Error::Invalid(format!("unknown CLI {other}"))),
                })
            }
            "--binary" => binary = Some(PathBuf::from(value()?)),
            "--model" => model = Some(value()?),
            "--effort" => effort = Some(value()?),
            "--arm" => {
                arms = vec![match value()?.as_str() {
                    "verifier" => Arm::Verifier,
                    "baseline" => Arm::Baseline,
                    other => return Err(Error::Invalid(format!("unknown arm {other}"))),
                }]
            }
            "--case" => only.push(value()?),
            "--jobs" => {
                jobs = value()?
                    .parse()
                    .map_err(|_| Error::Invalid("--jobs takes a number".into()))?
            }
            other if suite.is_none() && !other.starts_with('-') => {
                suite = Some(PathBuf::from(other))
            }
            other => return Err(Error::Invalid(format!("unknown option {other}\n{USAGE}"))),
        }
    }
    let provider = provider.ok_or_else(|| Error::Invalid(USAGE.into()))?;
    Ok(Options {
        suite: suite.ok_or_else(|| Error::Invalid(USAGE.into()))?,
        binary: binary.unwrap_or_else(|| PathBuf::from(agent::name(provider))),
        provider,
        model,
        effort,
        arms,
        only,
        jobs: jobs.max(1),
    })
}

pub fn eval(args: &[String]) -> Result<()> {
    if let Some(grading) = grading(args)? {
        return regrade(&grading.suite, &grading.files);
    }
    let options = Arc::new(parse(args)?);
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&options.suite)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("case.json").is_file())
        .filter(|path| {
            options.only.is_empty()
                || options
                    .only
                    .iter()
                    .any(|name| path.file_name().is_some_and(|file| file == name.as_str()))
        })
        .collect();
    cases.sort();
    require(!cases.is_empty(), "no cases (directories with case.json)")?;
    let work: Vec<(PathBuf, Arm)> = cases
        .iter()
        .flat_map(|case| options.arms.iter().map(|arm| (case.clone(), *arm)))
        .collect();
    let work = Arc::new(Mutex::new(work.into_iter()));
    let results = Arc::new(Mutex::new(Vec::<Value>::new()));
    let threads: Vec<_> = (0..options.jobs)
        .map(|_| {
            let (options, work, results) = (options.clone(), work.clone(), results.clone());
            std::thread::spawn(move || {
                loop {
                    let next = work.lock().unwrap().next();
                    let Some((case, arm)) = next else { break };
                    let result = run_case(&options, &case, arm);
                    println!("{result}");
                    results.lock().unwrap().push(result);
                }
            })
        })
        .collect();
    for thread in threads {
        let _ = thread.join();
    }
    let results = results.lock().unwrap();
    eprint!("{}", summary(&results));
    Ok(())
}

fn run_case(options: &Options, dir: &Path, arm: Arm) -> Value {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let started = Instant::now();
    let mut result = json!({
        "case": name,
        "arm": arm.name(),
        "cli": agent::name(options.provider),
        "model": options.model,
        "effort": options.effort,
    });
    let outcome = (|| -> Result<Value> {
        let case: Case = serde_json::from_str(&std::fs::read_to_string(dir.join("case.json"))?)?;
        let repo = std::env::temp_dir().join(format!("antislop-eval-{}", crate::id()));
        let outcome = judge(options, &case, dir, &repo, arm);
        let _ = std::fs::remove_dir_all(&repo);
        let (verdict, turns, tokens, unaccounted) = outcome?;
        let (correct, caught, named) = grade(&case, verdict.approved, &verdict.findings);
        Ok(json!({
            "defects": case.expect.defects.len(),
            "named": named,
            "scenario": case.scenario,
            "expected_approve": case.expect.approve,
            "approved": verdict.approved,
            "correct": correct,
            "caught": caught,
            "unaccounted": unaccounted,
            "turns": turns,
            "tokens_in": tokens.0,
            "tokens_out": tokens.1,
            "findings": verdict.findings,
        }))
    })();
    let map = result.as_object_mut().expect("object");
    match outcome {
        Ok(Value::Object(fields)) => map.extend(fields),
        Ok(_) => {}
        Err(error) => {
            map.insert("error".into(), Value::String(error.to_string()));
        }
    }
    map.insert(
        "seconds".into(),
        json!((started.elapsed().as_secs_f64() * 10.0).round() / 10.0),
    );
    result
}

/// Whether the verdict is the expected one, and whether a rejection names
/// the seeded defect: a rejection for the wrong reason is not a catch.
fn grade(case: &Case, approved: bool, findings: &[Value]) -> (bool, bool, usize) {
    let text = tasks::findings_text(findings).to_lowercase();
    let names = |words: &[String]| words.iter().any(|word| text.contains(&word.to_lowercase()));
    let named = case
        .expect
        .defects
        .iter()
        .filter(|words| names(words))
        .count();
    let caught = if case.expect.defects.is_empty() {
        names(&case.expect.mentions)
    } else {
        named == case.expect.defects.len()
    };
    (
        approved == case.expect.approve,
        !case.expect.approve && !approved && caught,
        named,
    )
}

/// Earlier results graded again against the suite's current expectations,
/// without running any agent.
fn regrade(suite: &Path, files: &[PathBuf]) -> Result<()> {
    let mut rows = vec![];
    for file in files {
        for line in std::fs::read_to_string(file)?.lines() {
            let mut row: Value = serde_json::from_str(line)?;
            let name = row["case"].as_str().unwrap_or_default().to_owned();
            let path = suite.join(&name).join("case.json");
            if row.get("error").is_none() && path.is_file() {
                let case: Case = serde_json::from_str(&std::fs::read_to_string(path)?)?;
                let approved = row["approved"] == true;
                let findings = row["findings"].as_array().cloned().unwrap_or_default();
                let (correct, caught, named) = grade(&case, approved, &findings);
                row["correct"] = json!(correct);
                row["caught"] = json!(caught);
                row["defects"] = json!(case.expect.defects.len());
                row["named"] = json!(named);
            }
            println!("{row}");
            rows.push(row);
        }
    }
    eprint!("{}", summary(&rows));
    Ok(())
}

type Tokens = (Option<u64>, Option<u64>);

fn judge(
    options: &Options,
    case: &Case,
    dir: &Path,
    repo: &Path,
    arm: Arm,
) -> Result<(Verdict, u32, Tokens, usize)> {
    copy_tree(&dir.join("base"), repo)?;
    git(repo, &["init", "--quiet"])?;
    let base = commit(repo, "base")?;
    let changes = if dir.join("change").is_dir() {
        copy_tree(&dir.join("change"), repo)?;
        let result = commit(repo, &case.task.title)?;
        Some(tasks::changes_text(
            &base,
            &result,
            &git(repo, &["diff", "--stat", &base, &result])?,
        ))
    } else {
        None
    };
    let project = repo.display().to_string();
    let mut spec = Spec {
        provider: options.provider,
        binary: options.binary.clone(),
        args: vec![],
        model: options.model.clone(),
        effort: options.effort.clone(),
        cwd: repo.to_path_buf(),
        add_dirs: vec![],
        access: Access::ReadOnly,
        resume: None,
    };
    let mut tokens: Tokens = (None, None);
    let mut turn = |spec: &Spec, prompt: &str| -> Result<agent::Outcome> {
        let cancel = AtomicBool::new(false);
        agent::run(spec, prompt, &cancel, &mut |event| {
            if let agent::Event::Usage { input, output } = event {
                let add = |sum: Option<u64>, more: Option<u64>| match (sum, more) {
                    (Some(sum), Some(more)) => Some(sum + more),
                    (sum, more) => sum.or(more),
                };
                tokens = (add(tokens.0, input), add(tokens.1, output));
            }
        })
    };
    match arm {
        Arm::Baseline => {
            let prompt = baseline_prompt(&project, &case.task, &case.report, changes.as_deref());
            let outcome = turn(&spec, &prompt)?;
            Ok((tasks::parse_verdict(&outcome.text), 1, tokens, 0))
        }
        Arm::Verifier => {
            let prompt =
                tasks::verifier_prompt(&project, &case.task, &case.report, changes.as_deref());
            let outcome = turn(&spec, &prompt)?;
            let obligations = tasks::obligations(&case.task);
            let mut verdict = tasks::parse_verdict(&outcome.text);
            let mut turns = 1;
            let missing = tasks::unaccounted(&verdict, &obligations);
            if !missing.is_empty() {
                // As the runtime does: one more turn to account for them.
                spec.resume = outcome.session;
                let outcome = turn(&spec, &tasks::coverage_followup(&missing))?;
                turns += 1;
                let earlier = std::mem::replace(&mut verdict, tasks::parse_verdict(&outcome.text));
                for (id, entry) in earlier.coverage {
                    verdict.coverage.entry(id).or_insert(entry);
                }
            }
            let unaccounted = tasks::unaccounted(&verdict, &obligations).len();
            tasks::settle(&mut verdict, &obligations);
            Ok((verdict, turns, tokens, unaccounted))
        }
    }
}

/// One agent asked plainly to review, as without antislop: the same task,
/// report and change, no named risks, obligations or checklists.
fn baseline_prompt(project: &str, task: &TaskSpec, report: &str, changes: Option<&str>) -> String {
    format!(
        "Review another agent's work. The project is your current working \
         directory: {project}. Work read-only.\n\n\
         Task: {}\nGoal: {}\n\n\
         The agent's report:\n<<<\n{report}\n>>>\n\n{}{}",
        task.title,
        task.goal,
        changes
            .map(|text| format!("{text}\n\n"))
            .unwrap_or_default(),
        tasks::VERDICT_FORMAT,
    )
}

fn summary(results: &[Value]) -> String {
    let mut out = String::from(
        "\narm              cases  errors  correct  defects caught  clean approved  named in multi  avg s  tokens out\n",
    );
    let mut groups: Vec<(String, String)> = results
        .iter()
        .map(|row| {
            (
                row["cli"].as_str().unwrap_or("?").to_owned(),
                row["arm"].as_str().unwrap_or("?").to_owned(),
            )
        })
        .collect();
    groups.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    groups.dedup();
    for (cli, arm) in groups {
        let rows: Vec<&Value> = results
            .iter()
            .filter(|row| row["cli"] == cli.as_str() && row["arm"] == arm.as_str())
            .collect();
        let arm = format!("{cli} {arm}");
        let errors = rows.iter().filter(|row| row.get("error").is_some()).count();
        let count = |key: &str| rows.iter().filter(|row| row[key] == true).count();
        let seeded = rows
            .iter()
            .filter(|row| row["expected_approve"] == false)
            .count();
        let clean = rows
            .iter()
            .filter(|row| row["expected_approve"] == true)
            .count();
        let clean_ok = rows
            .iter()
            .filter(|row| row["expected_approve"] == true && row["approved"] == true)
            .count();
        let named: u64 = rows.iter().filter_map(|row| row["named"].as_u64()).sum();
        let planted: u64 = rows.iter().filter_map(|row| row["defects"].as_u64()).sum();
        let seconds: f64 = rows.iter().filter_map(|row| row["seconds"].as_f64()).sum();
        let tokens: u64 = rows
            .iter()
            .filter_map(|row| row["tokens_out"].as_u64())
            .sum();
        out.push_str(&format!(
            "{arm:<16} {:>5}  {errors:>6}  {:>7}  {:>8}/{seeded:<5}  {clean_ok:>8}/{clean:<5}  {named:>8}/{planted:<5}  {:>5.0}  {tokens:>10}\n",
            rows.len(),
            count("correct"),
            count("caught"),
            seconds / rows.len() as f64,
        ));
    }
    out
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        // Left behind by running a case's code by hand; not part of the case.
        if entry.file_name() == "__pycache__" {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(Error::Invalid(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn commit(dir: &Path, message: &str) -> Result<String> {
    git(dir, &["add", "--all"])?;
    git(
        dir,
        &[
            "-c",
            "user.name=antislop",
            "-c",
            "user.email=antislop@localhost",
            "commit",
            "--quiet",
            "--no-verify",
            "--allow-empty",
            "-m",
            message,
        ],
    )?;
    git(dir, &["rev-parse", "HEAD"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_in_the_suite_reads() {
        let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("evals");
        let mut count = 0;
        for entry in std::fs::read_dir(&suite).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.join("case.json").is_file() {
                continue;
            }
            let case: Case =
                serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap())
                    .unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
            assert!(dir.join("base").is_dir(), "{}", dir.display());
            assert!(
                case.expect.approve
                    || !case.expect.mentions.is_empty()
                    || !case.expect.defects.is_empty(),
                "{}: a seeded case names what must be found",
                dir.display()
            );
            count += 1;
        }
        assert!(count >= 10);
    }

    #[test]
    fn the_baseline_names_no_obligations() {
        let task: TaskSpec = serde_json::from_value(json!({
            "id": "t1", "title": "Fix", "goal": "fix it", "kind": "implementation",
            "verifier": {"focus": "f", "risks": ["a risk"]}
        }))
        .unwrap();
        let plain = baseline_prompt("/p", &task, "done", None);
        assert!(!plain.contains("a risk") && !plain.contains("R1") && !plain.contains("C1"));
        let full = tasks::verifier_prompt("/p", &task, "done", None);
        assert!(full.contains("- R1: a risk") && full.contains("- C1: "));
    }
}
