// Executes the shipped extension with a small Pi API fixture. HTTP is real:
// discovery, scoped schemas and accepted results come from the Rust Gateway.
import assert from 'node:assert/strict';
import { existsSync, realpathSync, writeFileSync, appendFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
const argv = process.argv.slice(2);
if (argv.includes('--fixture-start-failure')) { console.error('Pi startup failed: missing backend'); process.exit(13); }
const adapter = argv[argv.indexOf('--extension') + 1];
let promptRead;
const lifecycle = argv.includes("--lifecycle");
let idle = true;
let aborted = false;
let prompts = [];
let wake;
let statusPublished = false;
if (argv.includes('--interactive')) {
    process.stdin.setRawMode(true);
    process.stdout.write('\x1b[?2004hPi fixture ready\n');
    promptRead = new Promise(resolve => {
        let input = '';
        const receive = bytes => {
            input += bytes.toString();
            if (input.includes('\x1b[201~') && input.endsWith('\r')) {
                assert(statusPublished, 'managed prompt arrived before current extension status');
                if (lifecycle) {
                    prompts.push(input);
                    idle = false;
                    input = '';
                    wake?.();
                } else {
                    process.stdin.off('data', receive);
                    resolve();
                }
            }
        };
        process.stdin.on('data', receive);
    });
}
const expected = JSON.parse(process.env.ANTISLOP_PI_EXPECTED);
const token = process.env.ANTISLOP_MCP_TOKEN;
const handlers = new Map();
let server;
const mode = process.env.PI_TEST_MODE;
const pi = {
    on(name, handler) { handlers.set(name, handler); },
    getThinkingLevel() {
        if (mode === 'non-reasoning' || mode === 'reasoning-off') return 'off';
        return mode === 'clamp' ? 'low' : expected.effort;
    },
    registerMcpServer(name, config) {
        assert.equal(name, expected.alias);
        assert.equal(config.exposure, 'direct');
        assert(config.headers.Authorization === `Bearer ${token}`);
        if (mode === 'register-error') throw new Error(`registration failed ${token}`);
        server = config;
    },
};
if (mode === 'missing-api') delete pi.registerMcpServer;
const { default: extension } = await import(adapter);
extension(pi);
assert.equal(process.env.ANTISLOP_MCP_TOKEN, undefined);
if (mode === 'provider-activity') {
    execFileSync('/bin/sh', ['-c', 'test "${ANTISLOP_MCP_TOKEN+x}" != x']);
    const result = handlers.get('tool_result')({
        content: [{type:'text', text:`runtime token ${token}`}],
        details: {raw: token},
        structuredContent: {unchanged: true},
    });
    assert.deepEqual(result, {
        content: [{type:'text', text:'runtime token [REDACTED]'}],
        details: {raw: '[REDACTED]'},
        structuredContent: {unchanged: true},
    });
}
const ctx = {
    isIdle: () => idle,
    hasPendingMessages: () => false,
    abort() { aborted = true; idle = true; },
    cwd: realpathSync(process.cwd()),
    model: { provider: expected.wrapper_model?.provider || 'fixture', id: expected.wrapper_model?.id || expected.model || 'model', reasoning: mode !== 'non-reasoning' },
    sessionManager: { getSessionFile: () => expected.session_file, getSessionId: () => 'pi-fixture' },
};
if (mode === 'wrong-cwd') ctx.cwd = '/';
if (mode === 'wrong-session') ctx.sessionManager.getSessionFile = () => '/foreign.jsonl';
if (mode === 'wrong-model') ctx.model.id = 'foreign';
if (mode === 'wrong-provider') ctx.model.provider = 'foreign';
if (lifecycle && !existsSync(expected.session_file)) writeFileSync(expected.session_file, JSON.stringify({type:'session', version:3, id:'fixture-session'}) + '\n');
if (!server) process.exit(0);
if (!argv.includes('--tools-first')) {
    handlers.get('session_start')({}, ctx);
    statusPublished = true;
}
if (mode === 'provider-activity') {
    idle = false;
    handlers.get('agent_start')({}, ctx);
    handlers.get('message_end')({message: {role: 'toolResult', stopReason: 'error', errorMessage: 'ordinary shell failure'}}, ctx);
    handlers.get('message_end')({message: {
        role: 'assistant', stopReason: 'error', errorMessage: `GPU timeout ${token}`,
    }}, ctx);
    handlers.get('agent_start')({}, ctx);
    handlers.get('message_end')({message: {role: 'assistant', stopReason: 'stop'}}, ctx);
}
if (mode) process.exit(0);
async function request(method, params) {
    const response = await fetch(server.url, {
        method: 'POST', headers: { ...server.headers, 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
    });
    assert.equal(response.status, 200);
    const reply = await response.json();
    if (reply.error) throw new Error(JSON.stringify(reply.error));
    return reply.result;
}
await request('initialize', { protocolVersion: '2025-06-18' });
const { tools } = await request('tools/list');
assert(tools.some(t => t.name === 'complete' && t.inputSchema.required.includes('turn')));
assert(!tools.some(t => t.name === 'review' || t.name === 'submit_plan'));
if (argv.includes('--tools-first')) {
    await new Promise(resolve => setTimeout(resolve, 600));
    handlers.get('session_start')({}, ctx);
    statusPublished = true;
}
if (lifecycle) {
    let reminderDeadline;
    while (true) {
        if (!prompts.length) await new Promise(resolve => { wake = resolve; });
        const prompt = prompts.shift();
        const turn = prompt.match(/turn = "([^"]+)"/)[1];
        aborted = false;
        if (prompt.includes('fixture-delay-start')) {
            console.log(`queued ${turn}`);
            await new Promise(resolve => setTimeout(resolve, 300));
        }
        handlers.get('agent_start')({}, ctx);
        appendFileSync(expected.session_file, JSON.stringify({type:'message', id:turn, message:{role:'user',content:prompt}}) + '\n');
        console.log(`working ${turn}`);
        if (prompt.includes('fixture-finish-without-tool')) {
            assert(prompt.includes('`mcp__antislop__complete`'));
            handlers.get('agent_end')({}, ctx);
            idle = true;
            reminderDeadline = setTimeout(() => {
                throw new Error('runtime did not remind Pi to submit its result');
            }, 5000);
            continue;
        }
        if (reminderDeadline) {
            clearTimeout(reminderDeadline);
            reminderDeadline = undefined;
            assert(prompt.includes('this MCP turn is still open'));
            assert(prompt.includes('`mcp__antislop__complete`'));
        }
        if (prompt.includes('fixture-exit')) process.exit(7);
        if (prompt.includes('fixture-disconnect')) console.log(`${expected.alias}: failed: connection reset by peer`);
        if (prompt.includes('fixture-malformed')) writeFileSync(process.env.ANTISLOP_PI_STATUS_PATH.replace(/status\.json$/, 'control.json'), '{');
        if (prompt.includes('fixture-wait')) {
            while (!aborted) await new Promise(resolve => setTimeout(resolve, 20));
        } else {
            if (prompt.includes('fixture-work-across-ticks')) {
                await new Promise(resolve => setTimeout(resolve, 400));
                assert.equal(aborted, false, 'previous cancellation aborted the following turn');
                assert.equal(idle, false, 'following turn became idle before completing');
            }
            const completion = {name:'complete', arguments:{turn, summary:'Completed the fixture task', report:'adapter result', commit_message:'test: lifecycle'}};
            if (prompt.includes('fixture-read-input')) {
                const inputTool = tools.find(t => t.name === 'read_turn_input');
                assert(inputTool?.inputSchema.properties.input);
                assert(!inputTool.inputSchema.required.includes('input'));
                assert(prompt.includes('`mcp__antislop__read_turn_input`'));
                assert(prompt.includes('mcp__antislop__current_turn'));
                const before = await request('tools/call', completion);
                assert.equal(before.isError, true);
                assert.match(before.content[0].text, /input-1/);
                const index = await request('tools/call', {name:'read_turn_input', arguments:{turn}});
                assert(!index.isError, index.content[0].text);
                assert.equal(JSON.parse(index.content[0].text).inputs[0].label, 'Fixture contract');
                const text = [];
                let complete;
                do {
                    const response = await request('tools/call', {name:'read_turn_input', arguments:{turn, input:'next'}});
                    assert(!response.isError, response.content[0].text);
                    const page = JSON.parse(response.content[0].text);
                    text.push(page.text);
                    complete = page.all_required_complete;
                    assert.equal(page.complete, page.cursor === null);
                } while (!complete);
                assert(text.length > 1);
                assert.equal(text.join(''), 'a👩‍💻é'.repeat(3000));
            }
            const accepted = await request('tools/call', completion);
            assert.equal(JSON.parse(accepted.content[0].text).recorded, true);
            idle = true;
        }
        handlers.get('agent_end')({}, ctx);
    }
}
const stale = await request('tools/call', { name: 'complete', arguments: { turn: 'stale', report: 'bad', commit_message: 'test: stale' } });
assert.equal(stale.isError, true);
if (argv.includes('--interactive')) {
    await promptRead;
    process.stdout.write('terminal prose is not a result\n');
    const release = argv[argv.indexOf('--release') + 1];
    while (!existsSync(release)) await new Promise(resolve => setTimeout(resolve, 20));
}
const accepted = await request('tools/call', { name: 'complete', arguments: { turn: 'current', report: 'adapter result', commit_message: 'test: adapter' } });
assert(accepted.content.some(c => c.type === 'text' && c.text.includes('recorded')));
assert(!accepted.isError);
console.log('adapter gateway result accepted');

if (argv.includes('--interactive')) await new Promise(() => {});
