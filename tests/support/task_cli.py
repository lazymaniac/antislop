# TASK_FIXTURE_PROTOCOL
import re as _lr, os as _lo, uuid as _lu
_fixture_prompt = prompt
_fixture_json = json.dumps
def _fixture_emit(answer):
    session = str(_lu.uuid4())
    if _lo.path.basename(sys.argv[0]) == 'codex':
        print(_fixture_json({'type':'thread.started','thread_id':session}))
        print(_fixture_json({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
        print(_fixture_json({'type':'turn.completed','usage':{}}))
    else:
        print(_fixture_json({'event':'init','conversation_id':session}))
        print(_fixture_json({'event':'result','result':{'conversation_id':session,'status':'SUCCESS','response':answer,'usage':{}}}))
    sys.exit(0)
def _fixture_block(tag, value):
    return '```' + tag + '\n' + _fixture_json(value) + '\n```'
_fixture_audit = _lr.search(r'BEGIN UNTRUSTED AUDITOR DATA[^\n]*\n(.*?)\nEND UNTRUSTED AUDITOR DATA\.', _fixture_prompt, _lr.S)
def _fixture_audit_response(answer):
    if not _fixture_audit or 'Audit response:' in answer:
        return answer
    data = json.loads(_fixture_audit.group(1))
    findings = data.get('findings_in_report_order') or []
    if data.get('error'):
        lines = ['Audit status: unaudited - the fixture audit failed']
    elif not findings:
        lines = ['Audit status: complete - the fixture audit approved the result']
    else:
        lines = ['Audit response:'] + ['finding %d: resolved - the fixture accepts its result' % (n + 1) for n in range(len(findings))]
    return answer.rstrip() + '\n\n' + '\n'.join(lines)
def _fixture_answer(answer):
    answer = _fixture_audit_response(answer)
    match = _lr.search(r'```plan\s*\n(.*?)\n```',answer,_lr.S)
    if match:
        payload = json.loads(match.group(1))
        for task in payload['tasks']:
            task.setdefault('acceptance_criteria',[{'id':'C1','text':task['goal']}])
        return answer[:match.start()] + _fixture_block('plan',payload) + answer[match.end():]
    return answer
def _fixture_dumps(value,*args,**kwargs):
    if isinstance(value,dict):
        if value.get('type')=='item.completed' and value.get('item',{}).get('type')=='agent_message':
            value['item']['text']=_fixture_answer(value['item']['text'])
        if value.get('event')=='result' and isinstance(value.get('result'),dict):
            if isinstance(value['result'].get('response'),str): value['result']['response']=_fixture_answer(value['result']['response'])
    return _fixture_json(value,*args,**kwargs)
json.dumps = _fixture_dumps
