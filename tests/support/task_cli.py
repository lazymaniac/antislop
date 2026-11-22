# TASK_FIXTURE_PROTOCOL
import re as _lr, os as _lo, uuid as _lu
_fixture_prompt = prompt
_fixture_json = json.dumps
def _fixture_tool(name, **arguments):
    import urllib.request as _fixture_http
    if name != 'current_turn':
        arguments['turn'] = _lo.environ['FIXTURE_TURN']
    body = {'jsonrpc':'2.0','id':32,'method':'tools/call','params':{'name':name,'arguments':arguments}}
    request = _fixture_http.Request(_lo.environ['ANTISLOP_MCP_URL'], _fixture_json(body).encode(), {'Authorization':'Bearer '+_lo.environ['ANTISLOP_MCP_TOKEN'],'Content-Type':'application/json'})
    result = json.load(_fixture_http.urlopen(request, timeout=30))['result']
    assert not result.get('isError'), result
    return json.loads(result['content'][0]['text'])
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
    if 'Audit response:' in answer:
        return answer
    if _fixture_audit:
        data = json.loads(_fixture_audit.group(1))
    elif 'AUTHORITATIVE AUDIT RESPONSE INSTRUCTIONS:' in _fixture_prompt:
        reference = _lr.search(r'- audit_ref: (audit:[0-9]+)', _fixture_prompt)
        if not reference:
            return answer
        data = _fixture_tool('read_audit', audit_ref=reference.group(1))['verdict']
    else:
        return answer
    findings = data.get('findings_in_report_order') or data.get('findings') or []
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

def _fixture_contract(task):
    """Read the reviewed contract through MCP instead of scraping prompt graphs."""
    args = {'task':task}
    text = ''
    while True:
        page = _fixture_tool('read_task_contract', **args)
        text += page['text']
        if page['complete']:
            break
        args['cursor'] = page['cursor']
    fields = list(_lr.finditer(r'^- ([a-z_]+):(?: ([^\n]*))?\n', text, _lr.M))
    contract = {}
    for index, match in enumerate(fields):
        key, value = match.groups()
        tail = text[match.end():fields[index+1].start() if index+1<len(fields) else len(text)]
        if key in ('depends_on','outputs'):
            value = _lr.findall(r'^  - (.*)$', tail, _lr.M)
        elif key in ('acceptance_criteria','result_inputs'):
            value = [dict(_lr.findall(r'^    - ([a-z_]+): (.*)$', entry, _lr.M)) for entry in tail.split('  - Entry:\n')[1:]]
        elif key == 'reviewer':
            continue
        elif key in ('deferred','quality_gate','integration_batch'):
            value = value == 'true'
        elif key in ('checkout_name','options_for','delivery','section','summary') and value == 'none':
            value = None
        elif tail:
            value = (value or '') + '\n' + '\n'.join(line.removeprefix('  ') for line in tail.rstrip('\n').splitlines())
        contract[key] = value
    return contract
