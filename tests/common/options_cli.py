#!/usr/bin/env python3
import glob, json, os, re, sys
prompt = sys.argv[-1]
root = os.path.dirname(sys.argv[0])
def record_call(call):
    with open(os.path.join(root, 'calls.jsonl'), 'a') as log:
        log.write(json.dumps(call)+'\n')

settings = json.load(open(os.path.join(root, 'settings.json')))
if os.path.basename(sys.argv[0]) == 'codex':
    answer = 'done' if '[runtime]' in prompt else '```plan\n'+json.dumps(settings['plan'])+'\n```'
else:
    scenario = settings.get('scenario', '')
    previous = sys.argv[sys.argv.index('--conversation')+1] if '--conversation' in sys.argv else ''
    worker = 'You are a worker' in prompt or previous.startswith('worker:')
    task = re.search(r'Task id: ([^\n]+)', prompt) if worker else re.search(r'Task: ([^\n]+)', prompt)
    task = task.group(1) if task else previous.split(':', 1)[-1]
    session = ('worker:' if worker else 'verifier:') + task
    counter_name = re.sub(r'[^A-Za-z0-9_-]', '_', session)
    count_file = os.path.join(root, counter_name + '-count')
    count = int(open(count_file).read()) + 1 if os.path.exists(count_file) else 1
    open(count_file, 'w').write(str(count))
    record_call({'worker':worker,'task':task,'prompt':prompt,'args':sys.argv[1:-1],'pid':os.getpid(),'count':count})
    if task == 'w' and count == 1:
        if scenario == ('resume-worker' if worker else 'resume-verifier'):
            print(json.dumps({'event':'init','conversation_id':session}), flush=True)
            print('login required: deliberate lifecycle fixture interruption', file=sys.stderr)
            sys.exit(1)
        if worker and scenario in ('mail-new', 'mail-existing'):
            import time
            open(os.path.join(root, 'mail-ready'), 'w').close()
            deadline = time.monotonic() + 15
            while not os.path.exists(os.path.join(root, 'mail-release')):
                if time.monotonic() > deadline:
                    raise RuntimeError('mail was not released by test')
                time.sleep(0.01)
    if worker:
        if task.endswith('-options') or task == 'shared':
            answer = settings['report']
        elif ('Work read-only; do not modify anything.' in prompt
              or any(t['id'] == task and not t.get('writes', False) for t in settings['plan']['tasks'])):
            answer = 'Inspected files read-only.'
        else:
            implementation = 'B' if scenario == 'reject-once' and count == 1 else settings['implementation']
            if implementation == 'selected':
                implementation = re.findall(r'^Selected: ([AB])$', prompt, re.M)[-1]
            open('result-'+task+'.txt','w').write(implementation)
            record_call({'written':task,'actual':open('result-'+task+'.txt').read()})
            answer = 'APPROVED. I complied with the selected variant. '+settings['claim']
    else:
        # A deterministic oracle for these fixtures only. Real semantic assessment
        # of a contradiction or departure belongs to the real verifier.
        files = {path:open(path).read() for path in glob.glob('result-*.txt')}
        actual = list(files.values())
        approved = not actual or all(value == 'A' for value in actual)
        if actual and not approved:
            # Parse the worker report delivered by runtime, never settings['claim'].
            # A claim is insufficient: check the cited repository contract and
            # compare its required format with the actual implementation bytes.
            report = prompt.split("The worker's report:\n<<<\n", 1)[-1].split('\n>>>', 1)[0]
            departure = re.search(r'Departure: (\{[^\n]+\})', report)
            if departure:
                evidence = json.loads(departure.group(1))
                path = evidence.get('evidence')
                if path == 'required-format.txt' and evidence.get('justification', '').strip():
                    required = open(path).read().strip()
                    approved = (required != 'A' and evidence.get('variant') == required
                                and all(value == required for value in actual))
        record_call({'verified':task,'files':files,'required':open('required-format.txt').read().strip(),'approved':approved})
        ids = re.findall(r'^- ([RCV][0-9]+): ', prompt, re.M)
        answer = '```verdict\n'+json.dumps({'approved':approved,'findings':[] if approved else [{'severity':'high','issue':'Implementation B contradicts Selected: A without evidence'}], 'coverage':{i:{'status':'ok','note':'fixture inspected'} for i in ids}})+'\n```'
if os.path.basename(sys.argv[0]) == 'codex':
    print(json.dumps({'type':'thread.started','thread_id':'planner'}))
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
    print(json.dumps({'type':'turn.completed','usage':{}}))
else:
    # A completed turn without a native session deliberately forces Core's
    # next mail turn through first=true, rather than changing the store in tests.
    result = {'status':'SUCCESS','response':answer,'usage':{}}
    if not (scenario == 'mail-new' and worker and task == 'w' and count == 1):
        print(json.dumps({'event':'init','conversation_id':session}))
        result['conversation_id'] = session
    print(json.dumps({'event':'result','result':result}))
