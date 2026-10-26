#!/usr/bin/env python3
# Test double for an interactive agent CLI. It behaves like the real ones as
# far as the daemon can tell: it runs in a terminal, takes a pasted prompt
# followed by Enter, and ends the turn by calling `finish_turn` on the MCP
# gateway it was launched with. The answer itself comes from the test's own
# deterministic script (`argv[1]`), which is run once per turn with
# the prompt as its last argument and may print the answer in any of the
# line-oriented shapes the earlier headless fixtures used.
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


def task_result(text, prompt):
    try:
        result = json.loads(text)
        if isinstance(result, dict) and result.get("type") in ("completed", "review", "needs_input"):
            return result
    except (ValueError, TypeError):
        pass
    if "Task role:" not in prompt:
        return {"type": "report", "text": text}
    pending = re.search(r"```integration_question\s*(.*?)```", text, re.S)
    if pending:
        body = json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ask_orchestrator","arguments":{"turn":turn_of_current,"request_key":"integration-choice","question":pending.group(1).strip()}}})
        req=urllib.request.Request(url,body.encode(),{"Authorization":"Bearer "+token,"Content-Type":"application/json"})
        response=json.load(urllib.request.urlopen(req,timeout=30))["result"]
        if response.get("isError"): raise RuntimeError(response)
        request=json.loads(response["content"][0]["text"])["request"]
        return {"type":"needs_input","request":request,"checkpoint":pending.group(1).strip()}
    if "Task role: verifier" in prompt:
        found = re.search(r"```verdict\s*(?:JSON\s*)?(.*?)```", text, re.S)
        verdict = json.loads(found.group(1)) if found else {"approved": True, "findings": [], "coverage": {}}
        if not verdict.get("coverage"):
            checks = re.search(r"Local check ids: (.*)", prompt)
            verdict["coverage"] = {key: {"status": "ok", "note": "Fixture source inspection"} for key in json.loads(checks.group(1))} if checks else {}
        checks = re.search(r"Local check ids: (.*)", prompt)
        if checks:
            allowed = set(json.loads(checks.group(1)))
            verdict["coverage"] = {key: value for key, value in verdict.get("coverage", {}).items() if key in allowed}
        for index, finding in enumerate(verdict.get("findings", [])):
            if isinstance(finding, dict):
                finding.setdefault("id", "fixture-finding-%d" % index)
                finding.setdefault("check", "V1")
                finding.setdefault("where", "fixture source")
        return {"type": "review", "report": text, "verdict": verdict}
    contract_match = re.search(r"Full task and working rules: (.*)", prompt)
    contract = json.loads(contract_match.group(1)) if contract_match else {}
    result = {"type": "completed", "summary": text.splitlines()[0] if text.splitlines() else "Completed", "report": text, "outputs": {key: text for key in contract.get("outputs", [])}}
    if contract.get("kind") == "options" or "Choose the best available approach" in prompt:
        result["selection"] = {"criteria": ["Satisfies local contract"], "alternatives": [
            {"id": "A", "approach": "Focused implementation", "evaluations": ["Satisfies the contract"], "eligible": True, "tradeoffs": "Local changes"},
            {"id": "B", "approach": "Broader restructuring", "evaluations": ["Higher change cost"], "eligible": True, "tradeoffs": "More coordination"}],
            "selected": (re.findall(r"Selected:\s*([AB])", text) or ["A"])[-1], "rationale": "The focused approach satisfies this local contract", "reconsider_when": "The contract or source evidence changes"}
    return result


def finish(turn, text, thread, prompt):
    global turn_of_current
    turn_of_current=turn
    params = {"name": "finish_turn", "arguments": {"turn": turn, "result": task_result(text,prompt)}}
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
        raise RuntimeError(reply["result"]["content"])


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
    urllib.request.urlopen(request, timeout=30).read()


def main():
    # Like the real CLIs, connect to the gateway while starting.
    rpc("initialize", {"protocolVersion": "2025-06-18"}, 1)
    rpc("notifications/initialized")
    rpc("tools/list", None, 2)
    fd = sys.stdin.fileno()
    tty.setraw(fd)
    # Like the real CLIs, ask the terminal to bracket pastes.
    sys.stdout.write("\x1b[?2004h")
    sys.stdout.flush()
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
        finish(turn, text, seen, prompt)


sys.exit(main())
