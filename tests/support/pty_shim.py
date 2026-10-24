#!/usr/bin/env python3
# Test double for an interactive agent CLI. It behaves like the real ones as
# far as the daemon can tell: it runs in a terminal, takes a pasted prompt
# followed by Enter, and ends the turn by calling `finish_turn` on the MCP
# gateway it was launched with. The answer itself comes from the test's own
# deterministic script, `<this file>.legacy`, which is run once per turn with
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

here = os.path.abspath(sys.argv[0])
legacy = here + ".legacy"
args = sys.argv[1:]
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


def finish(turn, text, thread):
    params = {"name": "finish_turn", "arguments": {"turn": turn, "text": text}}
    if thread:
        params["_meta"] = {"threadId": thread}
    body = json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": params})
    request = urllib.request.Request(
        url,
        body.encode(),
        {"Authorization": "Bearer " + token, "Content-Type": "application/json"},
    )
    urllib.request.urlopen(request, timeout=30).read()


def turn_of(prompt):
    match = re.search(r'turn = "([^"]+)"', prompt)
    return match.group(1) if match else None


def run_legacy(prompt):
    # The earlier fixtures were called like the headless CLIs.
    if name.endswith("codex") or "codex" in name:
        argv = [legacy] + extra + ["exec"]
        if resume:
            argv += ["resume", "-c", 'sandbox_mode="%s"' % sandbox] + model_args + [resume]
        else:
            argv += ["--json", "-C", os.getcwd(), "-s", sandbox] + model_args
    else:
        argv = [legacy] + extra
        if resume:
            argv += ["--resume", resume]
    argv.append(prompt)
    return subprocess.run(argv, capture_output=True, text=True, cwd=os.getcwd())


def main():
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
        done = run_legacy(prompt)
        text, seen = answer_of(done.stdout)
        if done.returncode != 0 or text is None:
            termios.tcsetattr(fd, termios.TCSADRAIN, termios.tcgetattr(fd))
            sys.stderr.write((done.stderr or done.stdout or "no answer") + "\n")
            return done.returncode or 1
        # Like the real CLIs, the next prompt continues this session.
        global resume
        resume = seen or resume or session
        finish(turn, text, seen if name.endswith("codex") else None)


sys.exit(main())
