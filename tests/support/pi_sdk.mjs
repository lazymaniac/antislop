// Opt-in compatibility test. ANTISLOP_TEST_PI_MODULE is the absolute installed
// pi-coding-agent/dist/index.js path; no package installation or user config edits.
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';
import { mkdirSync } from 'node:fs';
const sdk = await import(pathToFileURL(process.env.ANTISLOP_TEST_PI_MODULE).href);
const argv = process.argv.slice(2);
const adapter = argv[argv.indexOf('--extension') + 1];
const expected = JSON.parse(process.env.ANTISLOP_PI_EXPECTED);
const agentDir = join(dirname(process.env.ANTISLOP_PI_STATUS_PATH), 'sdk-agent');
mkdirSync(agentDir);
const settingsManager = sdk.SettingsManager.inMemory();
const loader = new sdk.DefaultResourceLoader({
    cwd: process.cwd(), agentDir, settingsManager,
    noExtensions: true, noSkills: true, noPromptTemplates: true, noThemes: true, noContextFiles: true,
    extensionFactories: [sdk.createMcpExtension()],
    additionalExtensionPaths: [adapter],
});
await loader.reload();
assert.equal(loader.getExtensions().errors.length, 0);
const { session } = await sdk.createAgentSession({
    cwd: process.cwd(), agentDir, settingsManager, resourceLoader: loader,
    sessionManager: sdk.SessionManager.open(expected.session_file),
});
try {
    await session.bindExtensions({ onError: e => { throw new Error(JSON.stringify(e)); } });
    const name = `mcp__${expected.alias}__complete`;
    // Pi connects direct MCP tools in the background. This deadline bounds the
    // test only; production readiness has no execution or idle deadline.
    const until = Date.now() + 15000;
    while (!session.agent.state.tools.some(t => t.name === name)) {
        if (Date.now() > until) throw new Error('Pi did not expose registered direct MCP tools');
        await new Promise(resolve => setTimeout(resolve, 20));
    }
    const tool = session.agent.state.tools.find(t => t.name === name);
    assert(tool.parameters.required.includes('turn'));
    assert(!session.agent.state.tools.some(t => t.name === `mcp__${expected.alias}__review`));
    const stale = await tool.execute('stale-call', { turn: 'stale', report: 'bad', commit_message: 'test: stale' });
    assert(stale.isError);
    assert(stale.structuredContent.isError);
    const images = session.agent.state.tools.find(t => t.name === `mcp__${expected.alias}__get_images`);
    const imageResult = await images.execute('image-call', { turn: 'current' });
    assert(imageResult.content.some(c => c.type === 'image' && c.mimeType === 'image/png'));
    assert(imageResult.structuredContent.content.some(c => c.type === 'image'));
    const accepted = await tool.execute('current-call', { turn: 'current', report: 'adapter result', commit_message: 'test: adapter' });
    assert(accepted.content.some(c => c.type === 'text' && c.text.includes('recorded')));
    console.log('real Pi adapter gateway result accepted');
} finally {
    session.dispose();
}
