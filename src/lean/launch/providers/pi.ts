// Private per-launch protocol v1. Values come only from antislop's launch environment.
// This module deliberately uses Pi's MCP implementation, including its tool schemas/results.
import { writeFileSync, renameSync, realpathSync, readFileSync } from 'node:fs';

export default function antislop(pi) {
    const token = process.env.ANTISLOP_MCP_TOKEN;
    // Pi's shell tools inherit process.env. Keep the gateway credential in the
    // adapter closure and the MCP client, rather than passing it to those tools.
    delete process.env.ANTISLOP_MCP_TOKEN;
    const expected = JSON.parse(process.env.ANTISLOP_PI_EXPECTED);
    const path = process.env.ANTISLOP_PI_STATUS_PATH;
    const redact = (error) => token ? String(error).split(token).join('[REDACTED]') : String(error);
    function publish(fields) {
        const temporary = `${path}.tmp`;
        writeFileSync(temporary, JSON.stringify({ version: 1, ...expected, ...fields }), { mode: 0o600 });
        renameSync(temporary, path);
    }
    let context;
    let request;
    let settledNonce;
    let generation = 0;
    let publishedIdle;
    let sequence = 0;
    let pendingFailure;
    const activity = {};
    function mark(kind, fields = {}) {
        activity[kind] = { generation, sequence: ++sequence, at_ms: Date.now(), ...fields };
    }
    function retry() {
        if (!pendingFailure || pendingFailure.retrying) return;
        pendingFailure.retrying = true;
        mark('retrying', { error: pendingFailure.error, failure_generation: pendingFailure.generation });
    }
    function inspect(_event, ctx) {
        context = ctx;
        try {
            const session_file = ctx.sessionManager.getSessionFile();
            const cwd = realpathSync(ctx.cwd);
            const model = ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : null;
            const effort = pi.getThinkingLevel();
            const actual = { session_file, cwd, model, effort, session_id: ctx.sessionManager.getSessionId() };
            if (session_file !== expected.session_file || cwd !== expected.cwd)
                throw new Error('Pi session file or cwd differs from the allocated session');
            const wrapper = expected.wrapper_model;
            const matches = wrapper
                ? ctx.model?.provider === wrapper.provider && ctx.model?.id === wrapper.id
                : !expected.model || expected.model === model || expected.model === ctx.model?.id;
            if (!matches)
                throw new Error(`Pi model mismatch: requested ${expected.model}, expected Pi ${wrapper ? `${wrapper.provider}/${wrapper.id}` : expected.model}, actual ${model}`);
            // Pi clamps every requested thinking level to off for non-reasoning
            // models, including models exposed through the Unsloth wrapper.
            const thinkingDisabled = ctx.model?.reasoning === false && effort === 'off';
            if (expected.effort && expected.effort !== effort && !thinkingDisabled)
                throw new Error(`Pi thinking mismatch: requested ${expected.effort}, actual ${effort}`);
            const idle = ctx.isIdle() && !ctx.hasPendingMessages();
            publishedIdle = idle;
            const acknowledged = request && generation > request.after_generation && idle;
            // Keep the acknowledgement readable, but consume its abort before any later agent_start.
            if (acknowledged) settledNonce = request.nonce;
            publish({ state: 'ready', actual, generation, idle, ack: acknowledged ? request.nonce : null, activity });
        } catch (error) {
            publish({ state: 'error', error: redact(error) });
        }
    }
    try {
        if (!token) throw new Error('Pi gateway credential is missing');
        if (typeof pi.registerMcpServer !== 'function')
            throw new Error('Pi lacks registerMcpServer; use a compatible Pi extension API (tested API: 1.5.2)');
        pi.registerMcpServer(expected.alias, {
            url: process.env.ANTISLOP_MCP_URL,
            headers: { Authorization: `Bearer ${token}` },
            exposure: 'direct',
        });
        // Tool results are also retained by Pi, independently of our event log.
        pi.on('tool_result', (event) => {
            const update = {};
            if (event.content.some(part => part.type === 'text' && part.text.includes(token))) {
                update.content = event.content.map(part =>
                    part.type === 'text' ? { ...part, text: redact(part.text) } : part);
            }
            for (const key of ['details', 'structuredContent']) {
                const encoded = JSON.stringify(event[key]);
                if (encoded?.includes(token)) update[key] = JSON.parse(redact(encoded));
                else if (key === 'structuredContent' && update.content && encoded !== undefined)
                    update[key] = event[key];
            }
            if (Object.keys(update).length) return update;
        });
        const timer = setInterval(() => {
            if (!context) return;
            try {
                let next;
                try { next = JSON.parse(readFileSync(path.replace(/status\.json$/, 'control.json'), 'utf8')); }
                catch (error) {
                    if (error.code !== 'ENOENT') throw error;
                    // agent_end can fire before Pi clears its busy state.
                    if (publishedIdle === false && context.isIdle() && !context.hasPendingMessages())
                        inspect({}, context);
                    return;
                }
                if (next.version !== 1 || next.launch_id !== expected.launch_id ||
                    typeof next.turn !== 'string' || !next.turn || typeof next.nonce !== 'string' ||
                    !next.nonce || typeof next.abort !== 'boolean' || !Number.isSafeInteger(next.after_generation) || next.after_generation < 0)
                    throw new Error('Invalid Pi control request');
                request = next;
                if (request.abort && request.nonce !== settledNonce && generation > request.after_generation &&
                    (!context.isIdle() || context.hasPendingMessages())) context.abort();
                inspect({}, context);
            } catch (error) {
                publish({ state: 'error', error: redact(error) });
            }
        }, 50);
        timer.unref();
        pi.on('agent_start', (event, ctx) => {
            generation += 1;
            if (!Number.isSafeInteger(generation)) {
                publish({state:'error', error:'Pi agent generation overflow'});
                return;
            }
            retry();
            mark('progress');
            inspect(event, ctx);
        });
        pi.on('message_start', (event, ctx) => {
            if (event.message?.role === 'assistant' && pendingFailure) {
                retry();
                inspect(event, ctx);
            }
        });
        pi.on('message_end', (event, ctx) => {
            const message = event.message;
            if (message?.role === 'assistant' && message.stopReason === 'error') {
                const error = redact(message.errorMessage || 'Pi reported a provider error');
                pendingFailure = { error, generation };
                mark('error', { error });
            } else if (message?.role === 'assistant' && !['aborted', 'length'].includes(message.stopReason)) {
                if (pendingFailure) mark('recovered', { failure_generation: pendingFailure.generation });
                pendingFailure = undefined;
                mark('progress');
            } else if (message?.role === 'toolResult') {
                mark('progress');
            } else {
                return;
            }
            inspect(event, ctx);
        });
        for (const event of ['session_start', 'session_switch', 'session_fork', 'model_select', 'agent_end'])
            pi.on(event, inspect);
    } catch (error) {
        publish({ state: 'error', error: redact(error) });
    }
}
