# LEDGER_FIXTURE_PROTOCOL
import re as _lr, os as _lo, uuid as _lu
_ledger_prompt = prompt
_ledger_json = json.dumps
_ledger_context = _ledger_prompt.rsplit('Run and ledger inputs (stable identities; keep their versions):\n',1)
_ledger_run = None
_ledger_entries = []
_ledger_sources = []
if len(_ledger_context) == 2:
    _ledger_lines = _ledger_context[1].splitlines()
    _ledger_run = json.loads(_ledger_lines[0])
    _ledger_sources = json.loads(_ledger_lines[1].removeprefix('Sources: '))
    _ledger_entries = json.loads(_ledger_lines[2].removeprefix('Entries: '))
def _ledger_emit(answer):
    session = str(_lu.uuid4())
    if _lo.path.basename(sys.argv[0]) == 'codex':
        print(_ledger_json({'type':'thread.started','thread_id':session}))
        print(_ledger_json({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
        print(_ledger_json({'type':'turn.completed','usage':{}}))
    else:
        print(_ledger_json({'event':'init','conversation_id':session}))
        print(_ledger_json({'event':'result','result':{'conversation_id':session,'status':'SUCCESS','response':answer,'usage':{}}}))
    sys.exit(0)
def _ledger_block(tag, value):
    return '```' + tag + '\n' + _ledger_json(value) + '\n```'
if _ledger_prompt.startswith('Act as requirements analyst before planning.'):
    entries = []
    if not _ledger_entries:
        source = _ledger_sources[0]
        marker = 'Full user source '+source['id']+':\n'
        full = _ledger_prompt.split(marker,1)[1].split('\n\nRun and ledger inputs',1)[0].split('\nFull user source ',1)[0].rstrip('\n')
        entries = [{'id':'fixture-requirement','kind':'requirement','source':{'source_id':source['id'],'start':0,'end':len(full.encode())},'interpretation':'Deliver the fixture user request','criteria':[{'id':'fixture.accept','text':'Independently inspect the actual fixture result','mandatory':True}]}]
    _ledger_emit(_ledger_block('ledger',{'run_id':_ledger_run['id'],'expected_spec_revision':_ledger_run['spec_revision'],'entries':entries}))
if _ledger_prompt.startswith('Independently compare both ledgers'):
    _ledger_emit(_ledger_block('verdict',{'approved':True,'findings':[]}))
if _ledger_prompt.startswith('Audit the integrated project'):
    _ledger_emit(_ledger_block('compliance',{'approved':False,'findings':[{'issue':'No integrated evidence supplied by this worker-only fixture'}],'evidence':[]}))
if _ledger_prompt.startswith('Verify this audit independently'):
    _ledger_emit(_ledger_block('compliance_verdict',{'approved':False,'note':'Worker-only fixture deliberately leaves integrated evidence open','assessments':[]}))
# Keep existing task-specific fixture logic independent of appended ledger hints.
if len(_ledger_context)==2:
    prompt = _ledger_context[0].rstrip()
_ledger_cache = _lo.path.join(_lo.path.dirname(sys.argv[0]), 'ledger-fixture-plan.json')
def _ledger_answer(answer):
    match = _lr.search(r'```plan\s*\n(.*?)\n```',answer,_lr.S)
    if match:
        payload = json.loads(match.group(1))
        if _ledger_run:
            payload['run_id']=_ledger_run['id']
            for task in payload['tasks']:
                task['requirement_ids']=[e['spec']['id'] for e in _ledger_entries if e['spec']['kind']=='requirement']
                task['criterion_ids']=[c['id'] for e in _ledger_entries for c in e['spec']['criteria']]
                task['plan_item_ids']=[e['spec']['id'] for e in _ledger_entries if e['spec']['kind']=='plan_item']
        with open(_ledger_cache,'w') as cache: cache.write(_ledger_json(payload))
        return answer[:match.start()] + _ledger_block('plan',payload) + answer[match.end():]
    return answer
def _ledger_dumps(value,*args,**kwargs):
    if isinstance(value,dict):
        if value.get('type')=='item.completed' and value.get('item',{}).get('type')=='agent_message':
            value['item']['text']=_ledger_answer(value['item']['text'])
        if value.get('event')=='result' and isinstance(value.get('result'),dict):
            if isinstance(value['result'].get('response'),str): value['result']['response']=_ledger_answer(value['result']['response'])
    return _ledger_json(value,*args,**kwargs)
json.dumps = _ledger_dumps
if _ledger_run and 'Create a task plan from the verified ledger' in _ledger_prompt and _lo.path.exists(_ledger_cache):
    with open(_ledger_cache) as cache: _ledger_payload=json.load(cache)
    _ledger_emit(_ledger_answer(_ledger_block('plan',_ledger_payload)))
