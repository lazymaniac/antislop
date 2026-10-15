//! Explicit native transport experiment; never part of the ordinary suite.
use architect::{
    config::{Config, ProviderKind, RoleKind, WorkKind},
    process::SupervisedChild,
    providers::{
        Provider,
        agy::{AgyProvider, NativeEvent},
        shared_cli,
    },
};
use serde_json::json;
use std::{
    fs,
    io::{BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Stdio,
};

#[test]
#[ignore = "explicit native usage; requires ANTISLOP_CONTEXT_PROBE_CONFIG"]
fn native_text_blocks_preserve_both_ends_above_the_single_string_ceiling() -> architect::Result<()>
{
    probe(false)
}

#[test]
#[ignore = "explicit native usage; requires ANTISLOP_CONTEXT_PROBE_CONFIG"]
fn native_scoped_context_reader_preserves_source_bytes_above_the_input_ceiling()
-> architect::Result<()> {
    probe(true)
}

fn probe(mcp: bool) -> architect::Result<()> {
    let configuration = PathBuf::from(
        std::env::var_os("ANTISLOP_CONTEXT_PROBE_CONFIG").ok_or_else(|| {
            architect::Error::Invalid("explicit probe configuration is required".into())
        })?,
    );
    let profile = Config::from_toml(&fs::read_to_string(configuration)?)?
        .resolve(WorkKind::Orchestration, RoleKind::Verifier)?;
    architect::require(
        profile.provider == ProviderKind::Agy,
        "probe requires an explicit Antigravity route",
    )?;
    let provider = AgyProvider {
        binary: Some(profile.binary.to_string_lossy().into()),
        managed_home: profile.managed_home.clone(),
        model: profile.model.clone(),
        effort: profile.native_effort.clone(),
        isolation: profile.isolation,
    };
    let capabilities = provider.probe()?;
    let home = profile
        .managed_home
        .clone()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| architect::Error::Invalid("CLI home missing".into()))?
        .canonicalize()?;
    let audited_cwd = std::env::temp_dir()
        .join(format!("architect-agy-{}", unsafe { libc::geteuid() }))
        .join(architect::hash(home.to_string_lossy().as_bytes()));
    let root = tempfile::Builder::new()
        .prefix("antislop-agy-block-probe-")
        .tempdir()?
        .keep();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let root = root.canonicalize()?;
    let agents = root.join(".agents/agents");
    fs::create_dir_all(&agents)?;
    fs::copy(
        audited_cwd.join(".agents/agents/antislop-stdio.md"),
        agents.join("antislop-stdio.md"),
    )?;
    let first = architect::id();
    let last = architect::id();
    let blocks = [
        format!(
            "This is a controlled input transport test. No tools or delegation. Ignore filler. Return the literal FIRST_SENTINEL and LAST_SENTINEL values in prefix and suffix. Never infer missing values. If either is missing return MISSING.\n{}\nFIRST_SENTINEL={first}\n",
            "filler ".repeat(17000)
        ),
        format!(
            "{}\nLAST_SENTINEL={last}\nReturn the two literal sentinels from the supplied text blocks using finish. No other tools.",
            "filler ".repeat(17000)
        ),
    ];
    let total: usize = blocks.iter().map(String::len).sum();
    assert!(total > architect::providers::agy::MAX_PROMPT_BYTES);
    assert!(
        blocks
            .iter()
            .all(|b| b.len() < architect::providers::agy::MAX_PROMPT_BYTES)
    );
    let schema = json!({"type":"object","properties":{"prefix":{"type":"string"},"suffix":{"type":"string"}},"required":["prefix","suffix"],"additionalProperties":false});
    let schema_file = root.join("schema.json");
    fs::write(&schema_file, serde_json::to_vec(&schema)?)?;
    let mut pinned = None;
    let input = if mcp {
        use architect::context::{Bundle, Reader, Scope};
        use std::os::unix::fs::OpenOptionsExt;
        let text = blocks.join("\n");
        let bundle = Bundle::new(
            Scope {
                run_id: "native-context-fixture".into(),
                spec_revision: 1,
                turn_id: architect::id(),
                input_hash: architect::hash("fixture"),
                prompt_hash: architect::hash(&text),
                schema_hash: architect::hash(serde_json::to_vec(&schema)?),
            },
            [("fixture-source".into(), text.clone())],
        )?;
        let bytes = bundle.bytes()?;
        let reader = Reader::from_bytes(&bytes, &architect::hash(&bytes))?;
        let bundle_file = root.join("bundle.json");
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&bundle_file)?
            .write_all(&bytes)?;
        let receipt_file = root.join("reads.jsonl");
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_antislop"));
        let mcp_configuration = json!({"mcpServers":{"antislop-context":{"command":binary,"args":["__context",bundle_file,reader.bundle_hash(),receipt_file]}}});
        fs::write(
            root.join(".agents/mcp_config.json"),
            serde_json::to_vec(&mcp_configuration)?,
        )?;
        let profile_path = agents.join("antislop-stdio.md");
        let definition = fs::read_to_string(&profile_path)?
            .replace("inheritMcp: false", "inheritMcp: true")
            .replace("Never use any other tool or delegate.","You may use only the pinned antislop-context MCP reader and finish. Never use other tools or delegate.");
        fs::write(profile_path, definition)?;
        let prompt = format!(
            "This is a controlled immutable context-read test. Use only the antislop-context MCP server read_context tool and finish. No delegation or other tools. The complete source is larger than the native input limit and is intentionally outside this short message. Start with the index document {}, bundle hash {}. Find fixture-source; fetch 512 bytes from offset 119000 and 512 bytes from offset {} in that source. Return the literal FIRST_SENTINEL and LAST_SENTINEL values as prefix and suffix through finish. Never infer missing values. These reads are not permission to access files or execute commands.",
            reader.index_hash(),
            reader.bundle_hash(),
            text.len() - 512
        );
        pinned = Some((reader, receipt_file));
        json!({"event":"user","message":{"content":prompt}})
    } else {
        json!({"event":"user","message":{"content":blocks.iter().map(|text|json!({"type":"text","text":text})).collect::<Vec<_>>()}})
    };
    let mut command = shared_cli::command("agy", profile.binary.to_str().unwrap())?;
    command
        .args([
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--json-schema",
        ])
        .arg(&schema_file)
        .args([
            "--agent",
            "antislop-stdio",
            "--disable-slash-commands",
            "--mode",
            "accept-edits",
            "--sandbox",
            "--print-timeout",
            "0",
        ])
        .env("HOME", &home)
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(model) = profile.model {
        command.arg("--model").arg(model);
    }
    if let Some(effort) = profile.native_effort {
        command.arg("--effort").arg(effort);
    }
    let source_bytes = total + usize::from(mcp);
    let input_hash = architect::hash(serde_json::to_vec(&input)?);
    let native_user_bytes = input["message"]["content"].as_str().map(str::len);
    println!(
        "Retained transport probe: {} ({} source bytes, native user bytes {:?})",
        root.display(),
        source_bytes,
        native_user_bytes
    );
    let mut child = SupervisedChild::spawn(&mut command)?;
    let mut stderr = child.child.stderr.take().unwrap();
    let diagnostics = std::thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
    let mut stdin = child.child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        stdin.write_all(serde_json::to_string(&input).unwrap().as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    });
    let mut reader = BufReader::new(child.child.stdout.take().unwrap());
    let mut native = None;
    let mut finished = false;
    let mut result = None;
    while let Some(frame) = architect::transport::read::<serde_json::Value>(&mut reader)? {
        let step = &frame["step_update"];
        if mcp
            && frame["event"] == "step_update"
            && (step["step_type"] == "mcp_tool" || step["tool_name"] == "call_mcp_tool")
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut capture = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(root.join("native-context-tools.jsonl"))?;
            serde_json::to_writer(&mut capture, &frame)?;
            capture.write_all(b"\n")?;
            architect::require(
                native.as_deref() == step["conversation_id"].as_str()
                    && result.is_none()
                    && step.get("subagent_info").is_none(),
                "invalid native MCP step scope",
            )?;
            println!(
                "Observed scoped MCP step: type={} tool={}",
                step["step_type"], step["tool_name"]
            );
            if step["state"] == "DONE"
                && step["tool_info"].get("error").is_some_and(|v| !v.is_null())
            {
                let diagnostic = serde_json::to_string(&step["tool_info"]["error"])?;
                println!(
                    "Context tool refusal: invalid_arguments={} wrong_bundle={} unknown_document={} invalid_offset={} invalid_size={}",
                    diagnostic.contains("Invalid context read arguments"),
                    diagnostic.contains("another bundle"),
                    diagnostic.contains("outside the pinned bundle"),
                    diagnostic.contains("invalid context byte offset"),
                    diagnostic.contains("page size")
                );
                println!(
                    "Argument field types: {}",
                    step["tool_info"]["parameters"]
                        .as_object()
                        .map(|o| o
                            .iter()
                            .map(|(k, v)| (
                                k,
                                if v.is_string() {
                                    "string"
                                } else if v.is_number() {
                                    "number"
                                } else if v.is_object() {
                                    "object"
                                } else {
                                    "other"
                                }
                            ))
                            .collect::<std::collections::BTreeMap<_, _>>())
                        .map(|v| serde_json::to_string(&v).unwrap())
                        .unwrap_or_default()
                );
                return Err(architect::Error::Invalid(
                    "native context read refused; experiment stopped without retries".into(),
                ));
            }
            continue;
        }
        match architect::providers::agy::normalize(&frame)? {
            NativeEvent::Init { id, cwd, agent } => {
                println!("Observed native session: {id}");
                architect::require(
                    native.is_none()
                        && std::path::Path::new(&cwd) == root
                        && agent == "antislop-stdio",
                    "invalid probe initialization",
                )?;
                native = Some(id);
            }
            NativeEvent::Step { id, finish } => {
                architect::require(
                    result.is_none() && native.as_ref() == Some(&id),
                    "invalid probe step order",
                )?;
                finished |= finish;
            }
            NativeEvent::Result {
                id,
                output,
                schema: echoed,
                ..
            } => {
                architect::require(
                    result.is_none()
                        && finished
                        && native.as_ref() == Some(&id)
                        && echoed == schema,
                    "invalid probe result",
                )?;
                result = Some(output);
            }
            NativeEvent::Failed { kind, .. } => {
                let error = serde_json::to_string(&frame["result"]["error"])?;
                let diagnosis = json!({"kind":kind,"executor_construction":error.contains("construct executor"),"missing_tool_converter":error.contains("no tool converter"),"unknown_tool":error.contains("unknown tool"),"mcp_related":error.to_lowercase().contains("mcp")});
                println!("Native failure metadata: {diagnosis}");
                fs::write(
                    root.join("failure.json"),
                    serde_json::to_vec_pretty(&diagnosis)?,
                )?;
                return Err(architect::Error::Invalid(
                    "native context probe failed; raw diagnostics are not retained".into(),
                ));
            }
        }
    }
    writer.join().unwrap()?;
    let status = child.child.wait()?;
    let diagnostic_bytes = diagnostics.join().unwrap()?;
    let output =
        result.ok_or_else(|| architect::Error::Invalid("probe has no structured result".into()))?;
    let preserved = output["prefix"] == first && output["suffix"] == last;
    let mut authenticated_reads = 0;
    if let Some((reader, receipt_file)) = pinned {
        for line in fs::read_to_string(receipt_file)?.lines() {
            let receipt: architect::context::Receipt =
                architect::transport::decode(line.as_bytes())?;
            let page = reader.read(
                &architect::context::Read {
                    bundle_hash: receipt.bundle_hash.clone(),
                    document_hash: receipt.document_hash.clone(),
                    offset: receipt.offset,
                    max_bytes: (receipt.end - receipt.offset).max(4),
                },
                receipt.sequence,
            )?;
            architect::require(
                page.receipt == receipt && receipt.sequence == authenticated_reads + 1,
                "invalid actual native context receipt",
            )?;
            authenticated_reads += 1;
        }
        architect::require(
            authenticated_reads >= 3,
            "native model did not independently read the index and both source pages",
        )?;
    }
    let report = json!({"version":capabilities.version,"native_session":native,"source_bytes":source_bytes,"native_user_bytes":native_user_bytes,"input_json_hash":input_hash,"transport":if mcp{"pinned_context_mcp"}else{"text_blocks"},"authenticated_reads":authenticated_reads,"complete_sentinels":preserved,"clean_exit":status.success(),"diagnostic_bytes":diagnostic_bytes,"output_hash":architect::hash(serde_json::to_vec(&output)?)});
    fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    architect::require(
        status.success() && preserved,
        "native text-block transport did not preserve both sentinels",
    )
}
