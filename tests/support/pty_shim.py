#!/usr/bin/env python3
# Test double for an interactive agent CLI. It behaves like the real ones as
# far as the daemon can tell: it runs in a terminal, takes a pasted prompt
# followed by Enter, and ends the turn by calling `finish_turn` on the MCP
# gateway it was launched with. The answer itself comes from the test's own
# deterministic script (`argv[1]`), which is run once per turn with
# the prompt as its last argument and may print the answer in any of the
# line-oriented shapes the earlier headless fixtures used. A ```plan block in
# an answer becomes a `submit_plan` call, as a real planner would make it.
import json
import os
import re
import subprocess
import sys
import termios
import tty
import urllib.request

here = os.path.abspath(sys.argv[1])
fixture = here
args = sys.argv[2:]
name = os.path.basename(here)

url = token = None
resume = None
for i, arg in enumerate(args):
    if arg == "--mcp-config":
        server = json.loads(args[i + 1])["mcpServers"]["antislop"]
        url = server["url"]
        token = server["headers"]["Authorization"].split()[1]
    elif arg.startswith("mcp_servers.antislop.url="):
        url = json.loads(arg.split("=", 1)[1])
    elif arg in ("--resume", "--session", "--conversation") and i + 1 < len(args):
        resume = args[i + 1]
    elif arg == "resume" and i + 1 < len(args):
        resume = args[i + 1]
if "OPENCODE_CONFIG_CONTENT" in os.environ:
    server = json.loads(os.environ["OPENCODE_CONFIG_CONTENT"])["mcp"]["antislop"]
    url = server["url"]
    token = server["headers"]["Authorization"].split()[1]
if "GEMINI_CLI_SYSTEM_SETTINGS_PATH" in os.environ:
    with open(os.environ["GEMINI_CLI_SYSTEM_SETTINGS_PATH"]) as settings:
        server = json.load(settings)["mcpServers"]["antislop"]
    url = server["httpUrl"]
url = url or os.environ.get("ANTISLOP_MCP_URL")
token = token or os.environ.get("ANTISLOP_MCP_TOKEN")
session = resume or ("session-%d" % os.getpid())
sandbox = "read-only"
for i, arg in enumerate(args):
    if arg == "-s" and i + 1 < len(args):
        sandbox = args[i + 1]
    if arg == "--permission-mode" and i + 1 < len(args):
        sandbox = "danger-full-access" if args[i + 1] == "acceptEdits" else "read-only"


# The launcher's own arguments come first, before the flags the daemon adds.
markers = ("-c", "resume", "--mcp-config", "--agent")
cut = min([args.index(m) for m in markers if m in args] or [len(args)])
extra = args[:cut]
model_args = []
for i, arg in enumerate(args):
    if arg in ("-m", "--model") and i + 1 < len(args):
        model_args += ["-m", args[i + 1]]
    if arg.startswith("model_reasoning_effort="):
        model_args += ["-c", arg]
    if arg == "--effort" and i + 1 < len(args):
        model_args += ["-c", 'model_reasoning_effort="%s"' % args[i + 1]]


def answer_of(output):
    """The answer in the last line-oriented record that carries one."""
    text = None
    seen = None
    for line in output.splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if not isinstance(record, dict):
            continue
        if record.get("type") == "thread.started":
            seen = record.get("thread_id")
        item = record.get("item") or {}
        if record.get("type") == "item.completed" and item.get("type") == "agent_message":
            text = item.get("text")
        result = record.get("result")
        if record.get("type") == "result" and isinstance(result, str):
            text = result
            seen = record.get("session_id") or seen
        if record.get("event") == "result" and isinstance(result, dict):
            text = result.get("response")
            seen = result.get("conversation_id") or seen
        if record.get("event") == "init":
            seen = record.get("conversation_id") or seen
        if record.get("type") == "text" and isinstance(record.get("part"), dict):
            text = (text or "") + record["part"].get("text", "")
    return text, seen


def selection_args(selection):
    """`select` arguments from a fixture's selection, longer form or not."""
    return {
        "alternatives": [
            {"id": a["id"], "approach": a["approach"], "tradeoffs": a.get("tradeoffs") or "None noted"}
            for a in selection["alternatives"]
        ],
        "selected": selection["selected"],
        "rationale": selection["rationale"],
    }


def review_args(text, verdict, prompt):
    """`review` arguments from a fixture's verdict. The runtime derives the
    approval, so a rejection without a blocking finding gets one, and an
    approval keeps only findings that do not block."""
    checks = re.search(r"Local check ids: (.*)", prompt)
    allowed = json.loads(checks.group(1)) if checks else []
    # A fixture that names no coverage settles every check; one that gives
    # an empty coverage means to skip them.
    coverage = verdict.get("coverage")
    if coverage is None:
        coverage = {key: {"status": "ok", "note": "Fixture source inspection"} for key in allowed}
    coverage = {key: value for key, value in coverage.items() if key in allowed}
    findings = []
    for finding in verdict.get("findings", []):
        if not isinstance(finding, dict):
            continue
        check = finding.get("check")
        # Fixtures written for the earlier V1–V6 checks still name them; the
        # goal is the one check every task has.
        if check not in allowed:
            check = "goal"
        findings.append({
            "check": check,
            "severity": finding.get("severity") or "high",
            "issue": finding.get("issue") or "Fixture finding",
            "where": finding.get("where") or "fixture source",
        })
    blocking = ("critical", "high", "medium")
    if verdict.get("approved"):
        for finding in findings:
            if finding["severity"] in blocking:
                finding["severity"] = "low"
    else:
        settled = all(c.get("status") in ("ok", "n/a") for c in coverage.values())
        if settled and not any(f["severity"] in blocking for f in findings):
            if "goal" in coverage:
                coverage["goal"] = {"status": "issue", "note": "The fixture rejected the work"}
            else:
                findings.append({"check": "goal", "severity": "high", "issue": "The fixture rejected the work", "where": "fixture verdict"})
    return {"report": text, "checks": coverage, "findings": findings}


def ask(question):
    """Asks the orchestrator through `ask`; the reply it returns."""
    body = json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ask","arguments":{"turn":turn_of_current,"question":question}}})
    req=urllib.request.Request(url,body.encode(),{"Authorization":"Bearer "+token,"Content-Type":"application/json"})
    response=json.load(urllib.request.urlopen(req,timeout=60))["result"]
    if response.get("isError"): raise RuntimeError(response)
    return json.loads(response["content"][0]["text"])


def fenced(text, tag):
    """The last ```<tag> block of `text`, as a match."""
    blocks = list(re.finditer(r"```%s[ \t]*(?:JSON)?[ \t]*\n(.*?)```" % tag, text, re.S))
    return blocks[-1] if blocks else None


def without(text, block):
    return (text[: block.start()] + text[block.end() :]).strip()


def plain_review_args(text, verdict):
    """`review` arguments for a plan, audit or code reviewer, from the
    verdict block the fixture wrote; the runtime derives the approval."""
    findings = []
    for finding in verdict.get("findings", []):
        if isinstance(finding, str):
            finding = {"issue": finding}
        findings.append({
            "severity": finding.get("severity") or "high",
            "issue": finding.get("issue") or "Fixture finding",
            "where": finding.get("where") or finding.get("id") or "fixture review",
        })
    blocking = ("critical", "high", "medium")
    if verdict.get("approved") is True:
        for finding in findings:
            if finding["severity"] in blocking:
                finding["severity"] = "low"
    elif not any(f["severity"] in blocking for f in findings):
        findings.append({"severity": "high", "issue": text.strip() or "The fixture rejected it", "where": "fixture verdict"})
    return {"report": text.strip() or "Reviewed", "findings": findings}


def addressed_of(text):
    """The ```comments block's addressed comments, and the text without it."""
    block = fenced(text, "comments")
    if block is None:
        return None, text
    try:
        listed = json.loads(block.group(1)).get("addressed") or []
    except (ValueError, AttributeError):
        return None, without(text, block)
    return [{"id": item["id"], "how": item.get("how") or "addressed"} for item in listed], without(text, block)


def source_text(prompt, source_id):
    """The full text of a user source as the extraction prompt shows it."""
    marker = "Full user source " + source_id + ":\n"
    if marker not in prompt:
        return None
    rest = prompt.split(marker, 1)[1]
    return re.split(r"\n(?:Full user source |Interview question context |Interview summary |Run and ledger inputs)", rest, maxsplit=1)[0].rstrip("\n")


SUBMITTED_ENTRY = ("id", "interpretation", "qualifiers", "parent", "depends_on", "mandatory", "criteria", "task_ids")


def ledger_args(payload, prompt):
    """`submit_ledger` arguments from a fixture's ```ledger block: each byte
    range becomes the quote it covers."""
    entries = []
    for entry in payload.get("entries") or []:
        submitted = {key: entry[key] for key in SUBMITTED_ENTRY if key in entry and entry[key] is not None}
        if "quote" in entry:
            submitted["quote"] = entry["quote"]
        elif isinstance(entry.get("source"), dict):
            span = entry["source"]
            text = source_text(prompt, span.get("source_id", "")) or ""
            submitted["quote"] = text.encode()[span.get("start", 0) : span.get("end", 0)].decode(errors="replace")
        submitted.setdefault("criteria", [])
        entries.append(submitted)
    return {"entries": entries}


def audit_args(payload):
    """`submit_audit` arguments from a fixture's ```compliance block."""
    findings = [
        {"severity": f.get("severity") or "high", "issue": f.get("issue") or "Fixture finding", "where": f.get("where") or "audit"}
        for f in (payload.get("findings") or [])
    ]
    if payload.get("approved") is False and not findings:
        findings.append({"severity": "high", "issue": "The fixture rejected the audit", "where": "audit"})
    evidence = []
    for item in payload.get("evidence") or []:
        proposal = {"criterion_id": item["criterion_id"], "task_id": item["task_id"], "result": item.get("result") or ""}
        if item.get("artifact"):
            proposal["artifact"] = item["artifact"]
        evidence.append(proposal)
    return {"findings": findings, "evidence": evidence}


def audit_review_args(payload, prompt):
    """`submit_audit_review` arguments from a ```compliance_verdict block;
    each assessment names the proposal by the id the prompt gave it."""
    listed = re.search(r"Proposed evidence \(assess each by its id\):\n(.*)\n", prompt)
    ids = [item["id"] for item in json.loads(listed.group(1))] if listed else []
    assessments = []
    for index, assessment in enumerate(payload.get("assessments") or []):
        evidence = assessment.get("evidence")
        if isinstance(evidence, dict):
            evidence = evidence.get("id")
        if not isinstance(evidence, str):
            evidence = ids[index] if index < len(ids) else "E%d" % (index + 1)
        assessments.append({"evidence": evidence, "approved": bool(assessment.get("approved")), "note": assessment.get("note") or "Checked"})
    if payload.get("approved") is False:
        assessed = {a["evidence"] for a in assessments}
        for a in assessments:
            a["approved"] = False
        assessments += [{"evidence": i, "approved": False, "note": "The fixture rejected it"} for i in ids if i not in assessed]
    return {"note": payload.get("note") or "Reviewed", "assessments": assessments}


def words_result(text, prompt=""):
    """How a role without a task ends its turn with an answer written for the
    earlier fenced blocks."""
    for tool, tag in (("submit_ledger", "ledger"), ("submit_audit", "compliance"), ("submit_audit_review", "compliance_verdict")):
        if tool in tools:
            block = fenced(text, tag)
            try:
                payload = json.loads(block.group(1)) if block else {}
            except ValueError:
                payload = {}
            if not isinstance(payload, dict):
                payload = {}
            if tool == "submit_ledger":
                return tool, ledger_args(payload, prompt)
            if tool == "submit_audit":
                return tool, audit_args(payload)
            return tool, audit_review_args(payload, prompt)
    if "review" in tools:
        block = fenced(text, "verdict") or fenced(text, "json")
        try:
            verdict = json.loads(block.group(1)) if block else None
        except ValueError:
            verdict = None
        if not isinstance(verdict, dict):
            try:
                verdict = json.loads(text.strip())
            except ValueError:
                verdict = None
        if not isinstance(verdict, dict) or "approved" not in verdict:
            verdict = {"approved": False, "findings": []}
        return "review", plain_review_args(without(text, block) if block else text, verdict)
    if "ask_user" in tools:
        block = fenced(text, "question")
        if block is not None:
            question = json.loads(block.group(1))
            options = [o if isinstance(o, dict) else {"label": o} for o in question.get("options", [])]
            return "ask_user", {"context": question.get("context", ""), "question": question.get("question", ""), "options": options}
        block = fenced(text, "requirements")
        if block is not None and block.group(1).strip():
            return "submit_requirements", {"markdown": block.group(1).strip()}
        addressed, text = addressed_of(text)
        arguments = {"text": text or "Done"}
        if addressed is not None:
            arguments["addressed"] = addressed
        return "finish_turn", arguments
    return "finish_turn", {"text": text or "Done"}


def task_result(text, prompt):
    """The tool and arguments that end the turn with `text`."""
    try:
        result = json.loads(text)
    except (ValueError, TypeError):
        result = None
    if isinstance(result, dict) and result.get("type") == "completed":
        if result.get("selection"):
            return "select", selection_args(result["selection"])
        return "complete", {"summary": result["summary"], "report": result["report"]}
    if isinstance(result, dict) and result.get("type") == "review":
        return "review", review_args(result["report"], result["verdict"], prompt)
    if "Task role:" not in prompt:
        return words_result(text, prompt)
    pending = re.search(r"```integration_question\s*(.*?)```", text, re.S)
    if pending:
        reply = ask(pending.group(1).strip())
        if reply.get("state") != "answered":
            return "wait_for_answer", {"note": pending.group(1).strip()}
        text = text[: pending.start()] + "Answer: " + (reply.get("answer") or "") + text[pending.end() :]
    if "Task role: verifier" in prompt:
        found = re.search(r"```verdict\s*(?:JSON\s*)?(.*?)```", text, re.S)
        verdict = json.loads(found.group(1)) if found else {"approved": True, "findings": []}
        return "review", review_args(text, verdict, prompt)
    if "Kind: options" in prompt or "Choose the best available approach" in prompt:
        return "select", {
            "alternatives": [
                {"id": "A", "approach": "Focused implementation", "tradeoffs": "Local changes"},
                {"id": "B", "approach": "Broader restructuring", "tradeoffs": "More coordination"}],
            "selected": (re.findall(r"Selected:\s*([AB])", text) or ["A"])[-1],
            "rationale": "The focused approach satisfies this local contract",
        }
    return "complete", {"summary": text.splitlines()[0] if text.splitlines() else "Completed", "report": text}


PLANNED = ("id", "title", "goal", "kind", "depends_on", "scope", "writes", "section", "summary")


def planned_task(task):
    """A fixture's task contract as a planner submits it: what to do and
    where, and the requirements it covers in place of explicit links."""
    if not isinstance(task, dict):
        return task
    planned = {key: task[key] for key in PLANNED if key in task and task[key] is not None}
    covers = []
    for key in ("covers", "requirement_ids", "criterion_ids"):
        for item in task.get(key) or []:
            if item not in covers:
                covers.append(item)
    if covers:
        planned["covers"] = covers
    return planned


def plan_call(text, prompt):
    """`submit_plan` arguments for an answer ending in a ```plan block."""
    if "Task role:" in prompt:
        return None
    blocks = list(re.finditer(r"```plan[ \t]*\n(.*?)```", text, re.S))
    if not blocks:
        return None
    block = blocks[-1]
    document = (text[: block.start()] + text[block.end() :]).strip() or "Plan"
    try:
        tasks = json.loads(block.group(1)).get("tasks")
    except (ValueError, AttributeError):
        tasks = block.group(1)
    if isinstance(tasks, list):
        tasks = [planned_task(task) for task in tasks]
    return {"document": document, "tasks": tasks}


def call(params, thread):
    """Calls a gateway tool; the error text when it refuses the call."""
    if thread:
        # Codex names its thread in the call; Agy's bridge adds the conversation.
        params["_meta"] = {"sessionId" if "agy" in name else "threadId": thread}
    body = json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": params})
    request = urllib.request.Request(
        url,
        body.encode(),
        {"Authorization": "Bearer " + token, "Content-Type": "application/json"},
    )
    reply = json.loads(urllib.request.urlopen(request, timeout=30).read())
    if reply.get("result", {}).get("isError"):
        return reply["result"]["content"][0]["text"]
    return None


def finish(turn, text, thread, prompt):
    """Ends the turn with `text`; `(tool, error)` when the gateway refuses
    it, so the fixture can be asked again like an agent told what to fix."""
    global turn_of_current
    turn_of_current=turn
    addressed, words = addressed_of(text)
    # A question or a requirements summary goes before a plan the same
    # answer carries, as the interview withholds plans.
    asks = "ask_user" in tools and (fenced(text, "question") or fenced(text, "requirements"))
    plan = None if asks else plan_call(words, prompt)
    if plan is not None:
        tool, arguments = "submit_plan", dict(plan, turn=turn)
        if addressed is not None and "ask_user" in tools:
            arguments["addressed"] = addressed
    else:
        tool, arguments = task_result(text, prompt)
        arguments = dict(arguments, turn=turn)
    error = call({"name": tool, "arguments": arguments}, thread)
    if error is None:
        return None
    with open(here + ".rejections", "a") as record:
        record.write(json.dumps({"turn": turn, "tool": tool, "error": error}) + "\n")
    return tool, error


def finish_in_words(turn, text, thread, prompt, refused):
    """After a result refused for good: the words of a refused plan only; a
    role that may not end a turn that way fails it."""
    plan = plan_call(text, prompt)
    if plan is None:
        raise RuntimeError(refused[1])
    error = call({"name": "finish_turn", "arguments": {"turn": turn, "text": plan["document"]}}, thread)
    if error is not None:
        raise RuntimeError(error)


def turn_of(prompt):
    match = re.search(r'turn = "([^"]+)"', prompt)
    return match.group(1) if match else None


def run_fixture(prompt, turn):
    # The earlier fixtures were called like the headless CLIs.
    if name.endswith("codex") or "codex" in name:
        argv = [fixture] + extra + ["exec"]
        if resume:
            argv += ["resume", "-c", 'sandbox_mode="%s"' % sandbox] + model_args + [resume]
        else:
            argv += ["--json", "-C", os.getcwd(), "-s", sandbox] + model_args
    else:
        argv = [fixture] + extra
        if resume:
            argv += ["--conversation" if "agy" in name else "--resume", resume]
    argv.append(prompt)
    env = dict(os.environ, FIXTURE_HEADLESS="1", FIXTURE_TURN=turn, ANTISLOP_MCP_URL=url, ANTISLOP_MCP_TOKEN=token or "")
    return subprocess.run(argv, capture_output=True, text=True, cwd=os.getcwd(), env=env)


def rpc(method, params=None, ident=None):
    message = {"jsonrpc": "2.0", "method": method}
    if ident is not None:
        message["id"] = ident
    if params is not None:
        message["params"] = params
    request = urllib.request.Request(
        url,
        json.dumps(message).encode(),
        {"Authorization": "Bearer " + token, "Content-Type": "application/json"},
    )
    return json.loads(urllib.request.urlopen(request, timeout=30).read() or b"null")


tools = set()


def main():
    # The terminal is set up before connecting, as the real CLIs draw their
    # UI first: the runtime may type the prompt as soon as the gateway sees
    # the session, and a cooked terminal would turn its Enter into a newline,
    # or the paste would arrive unbracketed.
    fd = sys.stdin.fileno()
    tty.setraw(fd)
    sys.stdout.write("\x1b[?2004h")
    sys.stdout.flush()
    # Like the real CLIs, connect to the gateway while starting.
    rpc("initialize", {"protocolVersion": "2025-06-18"}, 1)
    rpc("notifications/initialized")
    listed = rpc("tools/list", None, 2)
    tools.update(tool["name"] for tool in listed["result"]["tools"])
    buffer = b""
    while True:
        chunk = os.read(fd, 65536)
        if not chunk:
            return 0
        buffer += chunk
        end = buffer.find(b"\x1b[201~")
        if end < 0 or b"\r" not in buffer[end:]:
            continue
        start = buffer.find(b"\x1b[200~")
        pasted = buffer[start + 6 : end].decode().replace("\r", "\n")
        buffer = b""
        turn = turn_of(pasted)
        prompt = pasted.split("\n\n---\nWhen this turn is complete", 1)[0]
        with open(here + ".prompts", "a") as record:
            record.write(json.dumps({"turn": turn, "prompt": prompt}) + "\n")
        done = run_fixture(prompt, turn)
        text, seen = answer_of(done.stdout)
        if done.returncode != 0 or text is None:
            termios.tcsetattr(fd, termios.TCSADRAIN, termios.tcgetattr(fd))
            sys.stderr.write((done.stderr or done.stdout or "no answer") + "\n")
            return done.returncode or 1
        # Like the real CLIs, the next prompt continues this session.
        global resume
        resume = seen or resume
        refused = finish(turn, text, seen, prompt)
        for _ in range(2):
            if refused is None:
                break
            # First, since fixtures cut the prompt at the ledger context.
            retry = "%s was refused: %s\n\n%s" % (refused[0], refused[1], prompt)
            done = run_fixture(retry, turn)
            again, _ = answer_of(done.stdout)
            if done.returncode != 0 or again is None:
                break
            text = again
            refused = finish(turn, text, seen, retry)
        if refused is not None:
            finish_in_words(turn, text, seen, prompt, refused)


sys.exit(main())
