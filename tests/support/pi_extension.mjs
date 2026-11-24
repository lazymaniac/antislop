// Executes the shipped extension with a small Pi API fixture. HTTP is real:
// discovery, scoped schemas and accepted results come from the Rust Gateway.
import assert from 'node:assert/strict';
import { existsSync, realpathSync, writeFileSync, appendFileSync } from 'node:fs';
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
        if (mode === 'register-error') throw new Error(`registration failed ${process.env.ANTISLOP_MCP_TOKEN}`);
        server = config;
    },
};
if (mode === 'missing-api') delete pi.registerMcpServer;
const { default: extension } = await import(adapter);
extension(pi);
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
            await request('tools/call', {name:'complete', arguments:{turn, summary:'Completed the fixture task', report:'adapter result', commit_message:'test: lifecycle'}});
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
