// Private per-launch protocol v1. Values come only from antislop's launch environment.
// This module deliberately uses Pi's MCP implementation, including its tool schemas/results.
import { writeFileSync, renameSync, realpathSync } from 'node:fs';

export default function antislop(pi) {
    const expected = JSON.parse(process.env.ANTISLOP_PI_EXPECTED);
    const token = process.env.ANTISLOP_MCP_TOKEN;
    const path = process.env.ANTISLOP_PI_STATUS_PATH;
    const redact = (error) => String(error).split(token).join('[REDACTED]');
    function publish(fields) {
        const temporary = `${path}.tmp`;
        writeFileSync(temporary, JSON.stringify({ version: 1, ...expected, ...fields }), { mode: 0o600 });
        renameSync(temporary, path);
    }
    function inspect(_event, ctx) {
        try {
            const session_file = ctx.sessionManager.getSessionFile();
            const cwd = realpathSync(ctx.cwd);
            const model = ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : null;
            const effort = pi.getThinkingLevel();
            const actual = { session_file, cwd, model, effort, session_id: ctx.sessionManager.getSessionId() };
            if (session_file !== expected.session_file || cwd !== expected.cwd)
                throw new Error('Pi session file or cwd differs from the allocated session');
            if (expected.model && expected.model !== model && expected.model !== ctx.model?.id)
                throw new Error(`Pi model mismatch: requested ${expected.model}, actual ${model}`);
            if (expected.effort && expected.effort !== effort)
                throw new Error(`Pi thinking mismatch: requested ${expected.effort}, actual ${effort}`);
            publish({ state: 'ready', actual });
        } catch (error) {
            publish({ state: 'error', error: redact(error) });
        }
    }
    try {
        if (typeof pi.registerMcpServer !== 'function')
            throw new Error('Pi lacks registerMcpServer; use a compatible Pi extension API (tested API: 1.5.2)');
        pi.registerMcpServer(expected.alias, {
            url: process.env.ANTISLOP_MCP_URL,
            headers: { Authorization: `Bearer ${token}` },
            exposure: 'direct',
        });
        for (const event of ['session_start', 'session_switch', 'session_fork', 'model_select', 'agent_start', 'agent_end'])
            pi.on(event, inspect);
    } catch (error) {
        publish({ state: 'error', error: redact(error) });
    }
}
