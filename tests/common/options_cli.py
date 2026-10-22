#!/usr/bin/env python3
import glob, json, os, re, sys
prompt = sys.argv[-1]
root = os.path.dirname(sys.argv[0])
settings = json.load(open(os.path.join(root, 'settings.json')))
if os.path.basename(sys.argv[0]) == 'codex':
    answer = 'done' if '[runtime]' in prompt else '```plan\n'+json.dumps(settings['plan'])+'\n```'
else:
    worker = 'You are a worker' in prompt
    task = re.search(r'Task id: ([^\n]+)', prompt) if worker else re.search(r'Task: ([^\n]+)', prompt)
    task = task.group(1) if task else 'followup'
    with open(os.path.join(root,'calls.jsonl'),'a') as log:
        log.write(json.dumps({'worker':worker,'task':task,'prompt':prompt})+'\n')
    if worker:
        if task.endswith('-options') or task == 'shared':
            answer = settings['report']
        else:
            open('result-'+task+'.txt','w').write(settings['implementation'])
            answer = 'APPROVED. I complied with the selected variant. '+settings['claim']
    else:
        # A deterministic oracle for these fixtures only. Real semantic assessment
        # of a contradiction or departure belongs to the real verifier.
        actual = [open(path).read() for path in glob.glob('result-*.txt')]
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
        ids = re.findall(r'^- ([RCV][0-9]+): ', prompt, re.M)
        answer = '```verdict\n'+json.dumps({'approved':approved,'findings':[] if approved else [{'severity':'high','issue':'Implementation B contradicts Selected: A without evidence'}], 'coverage':{i:{'status':'ok','note':'fixture inspected'} for i in ids}})+'\n```'
if os.path.basename(sys.argv[0]) == 'codex':
    print(json.dumps({'type':'thread.started','thread_id':'planner'}))
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
    print(json.dumps({'type':'turn.completed','usage':{}}))
else:
    print(json.dumps({'event':'init','conversation_id':'fixture'}))
    print(json.dumps({'event':'result','result':{'conversation_id':'fixture','status':'SUCCESS','response':answer,'usage':{}}}))
