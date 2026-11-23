// Executes the shipped extension with a small Pi API fixture. HTTP is real:
// discovery, scoped schemas and accepted results come from the Rust Gateway.
import assert from 'node:assert/strict';
import { existsSync, realpathSync } from 'node:fs';
const argv = process.argv.slice(2);
const adapter = argv[argv.indexOf('--extension') + 1];
let promptRead;
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
                process.stdin.off('data', receive);
                resolve();
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
    getThinkingLevel() { return mode === 'clamp' ? 'low' : expected.effort; },
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
    cwd: realpathSync(process.cwd()),
    model: { provider: 'fixture', id: 'model' },
    sessionManager: { getSessionFile: () => expected.session_file, getSessionId: () => 'pi-fixture' },
};
if (mode === 'wrong-cwd') ctx.cwd = '/';
if (mode === 'wrong-session') ctx.sessionManager.getSessionFile = () => '/foreign.jsonl';
if (mode === 'wrong-model') ctx.model.id = 'foreign';
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
