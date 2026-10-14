# Antislop graphical workspace — proposed visual system
A desktop workspace for a user conversing with an orchestrator, watching dynamically chosen specialist agents and reviewing their work. Existing production UI is a Rust terminal app; this is a proposal for a NEW graphical frontend, not a screenshot of running functionality.

## Visual direction
Quiet, polished dark workspace. Warm charcoal surfaces, restrained lavender accents, emerald observed activity, amber waiting states. Avoid rainbow/glow-heavy hacker dashboards. Canvas #101215; panel #181b20; elevated #22262d; border #303640; text #edf0f5; secondary #a6afbd; accent #b1a0ff; running #65d8aa; pending #e7be78. Sans-serif system font, monospaced technical identifiers only. 8px spacing rhythm, 12px cards, 16px panels, soft shadows. No existing logo/image assets; use Antislop product name as text, no invented logo symbol.

## Composition and flow
Compact project sidebar, clear top bar and stage strip. Left orchestrator conversation with comfortable message cards and composer; dominant center/right spatial agent graph, not a nested file list. Smooth curved labelled edges; distinguish supervision, verification, dependencies and messages. Selected node opens contextual inspector with Overview, Evidence and Comments. Floating Current step overlay in graph corner. Graph toolbar offers search, fit view and zoom. No decorative charts or meaningless metrics.

The user starts with a chat request. Orchestrator interviews first, then decides roles/provider/model/effort/task dependencies using predefined role system prompts. No required SPEC file or manual agent assignment. Send button and Ctrl+S work. Every node, edge, artifact, requirement and diff hunk exposes comments; inspector shows anchored comment and actual orchestrator reaction. Never imply a queued comment has already been acted upon. Runtime activity/status is observation, not animation-generated proof. Reviewed task/team proposals have concrete approval cards; installing plan is separate from executing work.

## Interaction/accessibility
Keyboard focus visible; reduced motion; readable contrast; tooltips explain compact controls. Graph pan/zoom, node selection and comment anchors. Status text accompanies color. Collapsible sidebar and inspector; narrow viewport switches between conversation and graph without tiny text. Subtle 120–180ms transitions, no constant distracting animation. Polish user-facing copy. Sample task: improve application onboarding. Mock data clearly belongs to a design preview, not live daemon execution.
