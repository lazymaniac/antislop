use super::*;

/// What a verifier checks besides its own focus, by the kind of work: only
/// the concerns of its worker's domain.
pub fn checklist(kind: WorkKind) -> &'static str {
    match kind {
        WorkKind::Orchestration => orchestration_checklist(),
        WorkKind::Planning => planning_checklist(),
        WorkKind::Context => context_checklist(),
        WorkKind::Requirements => requirements_checklist(),
        WorkKind::Options => options_checklist(),
        WorkKind::Architecture => architecture_checklist(),
        WorkKind::Dependencies => dependencies_checklist(),
        WorkKind::Implementation => implementation_checklist(),
        WorkKind::Refactoring => refactoring_checklist(),
        WorkKind::StaticAnalysis => static_analysis_checklist(),
        WorkKind::TestDesign => test_design_checklist(),
        WorkKind::Testing => testing_checklist(),
        WorkKind::Security => security_checklist(),
        WorkKind::Performance => performance_checklist(),
        WorkKind::Integration => integration_checklist(),
        WorkKind::Research => research_checklist(),
        WorkKind::Writing => writing_checklist(),
        WorkKind::Analysis => analysis_checklist(),
        WorkKind::Delivery => delivery_checklist(),
    }
}

fn orchestration_checklist() -> &'static str {
    "assignments and shared contracts consistent with current decisions; \
             dependent tasks start only when required inputs are available and \
             accepted; changed inputs trigger reassessment of dependent results \
             and verdicts; messages and verdicts match the current task, attempt \
             and input revision; cancellation reaches affected work and late \
             results do not revive superseded assignments; blockers have a \
             cause, responsible person or role and actionable next step; \
             independent work proceeds where possible; progress and completion \
             follow verified outcomes and agreed criteria"
}

fn planning_checklist() -> &'static str {
    "dependencies complete and correctly ordered; no intermediate state \
             or ordering assumed that the system does not guarantee; unknowns \
             established, not assumed; every requirement has a task delivering \
             its result and a way to verify fulfillment; each task's goal names \
             a recognizable result and says how to tell it is done; assigned \
             scopes, work kinds and permissions allow task goals to be met; \
             dependencies justified by required inputs without unnecessary \
             ordering; consequential assumptions resolved before costly \
             dependent commitments; external prerequisites have an explicit \
             acquisition path and timing"
}

fn context_checklist() -> &'static str {
    "every claim backed by files actually read; dependencies, contracts \
             and conventions that matter are covered; open unknowns named; \
             search scope and exclusions checked before claiming absence; \
             relevant reads complete despite output limits and pagination; \
             active implementation traced from the real entry point; generated \
             code distinguished from its authoritative source; instructions \
             scoped to affected paths inspected; conflicting evidence explained \
             or explicitly left unresolved"
}

fn requirements_checklist() -> &'static str {
    "every requirement of the source found, including ones in the middle; \
             each qualifier, applicability condition and scope kept; material \
             ambiguities resolved or explicitly left open; acceptance criteria \
             have observable outcomes and conditions that distinguish fulfillment; \
             contradictions named and resolved only by an authorized decision \
             or retained as open; mandates distinguished from preferences, \
             examples and suggestions using source evidence; current requirements \
             and criteria reflect changes and withdrawals without losing \
             constraints that still apply"
}

fn options_checklist() -> &'static str {
    "real alternatives considered under the same conditions; a variant \
             that misses an obligation is excluded before ranking; the choice \
             follows from the evidence; a result that shows a better variant \
             or breaks a premise is reported explicitly with evidence instead of silently reopening the choice, without automatically stopping the plan; 2–4 genuinely different realistic variants and explicit common criteria; \
             feasible no-change, reuse and minimal-change options considered or \
             their exclusion justified; material lifecycle costs compared over \
             the same time horizon; correlated criteria do not double count \
             advantages; unknowns distinguished from demonstrated strengths and \
             weaknesses; material trade-offs and disadvantages of the selected \
             option stated; an observable condition for reconsidering the \
             selection stated; no implementation, invented measurements or \
             copied repository contents; report ends with Selected: <id>"
}

fn architecture_checklist() -> &'static str {
    "fits the project's existing structure and idioms; the simplest \
             adequate design; no pattern or layer without a present need; no \
             policy duplicated across modules; each invariant has an explicit \
             owner and public interfaces enforce it; stable domain rules do not \
             depend on volatile implementation details; module boundaries keep \
             cohesive responsibilities together; failures stay within the \
             required availability boundaries; component contracts can be \
             verified with controlled dependencies and required integrations \
             are justified"
}

fn dependencies_checklist() -> &'static str {
    "an existing maintained library considered before custom code; \
             licenses of the actual dependency graph (vendored files included) \
             compatible; chosen license option and its obligations stated; \
             resolved dependencies support required toolchains and platforms; \
             enabled features and backends justified; native tools and system \
             libraries needed for building and linking stated; technical cost \
             of transitive dependencies assessed; assessed versions retrievable \
             and reproducible; required patches and upgrades have a feasible \
             maintenance path and explicit ownership"
}

fn implementation_checklist() -> &'static str {
    "does exactly what the goal asks, nothing more; no duplicated module \
             or needless layer; follows project conventions; edge cases and \
             combinations of inputs handled; tests exercise the change with \
             meaningful assertions; no refactoring the change needs left undone; \
             invalid input produces the required error outcome; branches match \
             decision rules; processing boundaries neither skip elements nor \
             process extras; conversions preserve required information and \
             meaning; operations preserve the intended object identity and data \
             scope; every exit path satisfies required effects and resource \
             release; completion requires confirmation of required effects; \
             inspect the worker's recorded checks and their coverage without running them"
}

fn refactoring_checklist() -> &'static str {
    "behaviour preserved, with evidence; structure actually improved; no \
             speculative abstraction; relevant code smells inspected in touched \
             code: long functions, oversized units, primitive obsession, long \
             parameter lists, data clumps, inconsistent alternative interfaces, \
             refused inherited contracts, repeated type or state branching, \
             temporary fields, divergent change, parallel hierarchies, shotgun \
             surgery, compensating comments, duplication, passive data holders, \
             dead code, underweight units, speculative generality, feature envy, \
             inappropriate intimacy, message chains, middle men and scattered \
             workarounds for incomplete libraries; inheritance-specific smells \
             assessed only where applicable; each transformation justified by \
             a concrete maintenance benefit rather than a size threshold; \
             when no change is needed, the report says \
             why instead of a cosmetic patch"
}

fn static_analysis_checklist() -> &'static str {
    "analyzers actually run with the project's configuration; warnings \
             count even when the exit code is 0; no suppression, rule weakening \
             or baseline change to make findings disappear; a finding that \
             predates the change but lies in the agreed scope still counts, and \
             a doubt about the scope is named, not settled silently"
}

fn test_design_checklist() -> &'static str {
    "scenarios follow from the requirements and the likely failures; \
             combinations of parameters that interact are covered, not only \
             each one separately or in pairs when three or more are coupled; \
             scenarios state initial conditions, stimuli and observable \
             contract-based outcomes; relevant state transitions and usage \
             sequences covered; duplicate, delayed and reordered events have \
             expected outcomes or explicitly unresolved semantics; significant \
             failure, interruption and resumption points covered where relevant; \
             critical concurrent orderings controllable without timing alone; \
             required side effects and forbidden changes specified; test levels \
             cross the boundaries needed to expose their target risks without \
             requiring every test level or possible interleaving for each change"
}

fn testing_checklist() -> &'static str {
    "tests have assertions that fail on the bug they target; they run \
             the real code path, not a mock of it; green runs actually observed"
}

fn security_checklist() -> &'static str {
    "threats relevant to the change modelled; findings reproduced, false \
             alarms separated; the fix closes the issue without new exposure"
}

fn performance_checklist() -> &'static str {
    "benchmark representative and compared to a baseline under the same \
             conditions; semantics unchanged; latency distributions checked \
             against requirements rather than averages alone; queue growth, \
             admission control and recovery under overload assessed where \
             relevant; sustained operation covers degradation and periodic \
             costs where relevant; cold start, first use and warmed operation \
             measured separately where relevant; comparisons include the full \
             scenario and relevant resource costs; claimed improvements exceed \
             measurement noise with evidence from repeated comparisons; \
             concurrency bottlenecks, retry amplification and resource use \
             after cancellation assessed where relevant"
}

fn integration_checklist() -> &'static str {
    "the combined change builds and passes checks as a whole; contracts \
             between changed modules agree; no regression; components connected \
             to actual flows from public entry points; adapters preserve data \
             meaning; failures propagate according to the end-to-end contract; \
             packaged artifacts work outside the source tree when packaging is \
             in scope; supported upgrade and mixed-version scenarios work when \
             applicable; required rollback restores a working system"
}

fn research_checklist() -> &'static str {
    "every source exists and supports the claim attributed to it; facts \
             separated from conclusions; sources current enough; source \
             credibility and applicability to the question assessed; original \
             materials inspected where accessible and access limitations \
             disclosed; counterevidence sought and source dependencies checked \
             without requiring multiple sources for every fact; conflicting \
             findings explained or explicitly unresolved; absence claims \
             justified beyond an empty search result; important research \
             questions mapped to evidence or explicit unknowns before stopping"
}

fn writing_checklist() -> &'static str {
    "factually correct against the sources and the project; consistent; \
             matches the brief"
}

fn analysis_checklist() -> &'static str {
    "data and assumptions stated; calculations recomputed independently; \
             units and denominators correct; conclusions follow from premises \
             and stay within the examined scope; hypotheses distinguished from \
             established facts by evidence that separates competing explanations; \
             methods and metrics answer the analytical question; conclusions \
             checked for sensitivity to assumptions and thresholds; reproducible \
             from stated inputs, versions, method and parameters"
}

fn delivery_checklist() -> &'static str {
    "the report matches what was actually done; limitations and open \
             risks stated; instructions work; delivered artifacts clearly \
             identified and accessible at the stated locations; prerequisites \
             and required user actions explicit; material behavior changes and \
             transition steps explained; verification scope and omitted checks \
             accurately reported; open issues have consequences and actionable \
             next steps; side effects of usage commands disclosed before execution"
}
