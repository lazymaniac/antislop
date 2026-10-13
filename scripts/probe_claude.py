#!/usr/bin/env python3
"""Explicit real-provider smoke test; tool-free sessions, metadata-only report."""
import json
import pathlib
import subprocess
import tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]

def turn(root,prompt,resume=None):
    schema=dict(type='object',properties=dict(answer=dict(type='integer')),required=['answer'],additionalProperties=False)
    argv=['claude','--safe-mode','--strict-mcp-config','--mcp-config','{"mcpServers":{}}','--tools','',
          '--no-chrome','--permission-mode','dontAsk','--permission-prompts','none','--print',
          '--output-format','stream-json','--verbose','--json-schema',json.dumps(schema),prompt]
    if resume:argv+=['--resume',resume]
    result=subprocess.run(argv,cwd=root,capture_output=True,text=True)
    events=[]
    for line in result.stdout.splitlines():
        try:events.append(json.loads(line))
        except ValueError:raise RuntimeError('malformed stream-json')
    session=next((e.get('session_id') for e in events if e.get('session_id')),None)
    final=next((e for e in reversed(events) if e.get('type')=='result'),{})
    if result.returncode or final.get('is_error'):raise RuntimeError('Claude turn failed; raw diagnostics intentionally not retained')
    output=final.get('structured_output')
    if output is None:
        try:output=json.loads(final.get('result',''))
        except ValueError:raise RuntimeError('structured response unavailable')
    return session,output,[e.get('type') for e in events]

def main():
    with tempfile.TemporaryDirectory(prefix='architect-claude-probe-') as root:
        session,output,events=turn(root,'Remember 37. Return JSON answer=37. No tools.')
        assert session and output['answer']==37
        resumed,output,other=turn(root,'Return the remembered number plus 5 as JSON.',session)
        assert resumed==session and output['answer']==42
    report=dict(version='2.1.296',structured_output='observed',streaming='observed',exact_id_resume='observed',context_retained='observed',
                tool_restrictions='--tools empty; safe-mode; strict empty MCP config; dontAsk; no permissions prompts',
                mid_turn_steering='unverified',cancel='unverified',event_types=sorted(set(events+other)))
    (ROOT/'docs/m0/claude-session.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
if __name__=='__main__':main()
