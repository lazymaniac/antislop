# Prompt behavior evaluation

Contract regression tests and model behavior evaluation answer different questions.
The Rust and CLI fixture tests verify effective options policy, generated graph
ordering, result validation, evidence retention and the user-facing projection.
They do not establish that a language model decomposes work or reviews code well.

Use the cases below when changing a role prompt or switching a configured model.
Run each in a separate disposable Git repository and antislop conversation, using
the normal CLI permission policy. Keep the original inputs, assembled turn prompt,
provider/model identity, structured tool results, actual checks and final diff in
durable runtime storage. Do not edit a response to make it match the rubric.
Compare the same inputs before and after a prompt change. Record missing inputs
and failed runs rather than excluding them from the results.

| Case | Input and setup | Required observable result |
| --- | --- | --- |
| Broad feature | Add image ingestion, durable message linkage, conversation-specific drafts and provider delivery. Storage is a producer shared by independently usable consumers. Select small-change. | Authored tasks expose usable producer/consumer boundaries and cover every requested behavior. Generated options/checks do not count as implementation decomposition. No arbitrary required task count: assess actual boundaries and dependencies. |
| Cohesive fix | Reject a single invalid parser value and add the focused regression. | One cohesive implementation is acceptable. No unrelated architecture, research or documentation tasks. |
| Options off | Run the same code task with local options disabled, security-fix, and no-quality-gates. | Neither worker nor reviewer waits for a selection; ordinary acceptance and dependencies still apply. No generated options are requested. |
| One feasible choice | Existing serialized format and public API must remain unchanged; only the existing validation hook can implement the requested rejection. | A single feasible alternative is valid with the constraints explained. No fabricated second approach or external blocker. |
| Exact review target | Create commit A containing a defect and commit B fixing it, then request review of A. | Every review task retains A as its target and examines A rather than silently evaluating the current checkout. If A is unavailable, the missing target is reported. |
| Interrupted continuation | Interrupt a writer after a retained checkpoint, then resume it. | Existing work, decisions, evidence and current acceptance criteria survive; finished work is not recreated. Reviewers remain read-only after resumption. |
| Late serious finding | Round one names one report omission. In round two the report fixes it, but source inspection reveals another serious omission inside the original scope. | The new omission retains its actual severity and can block. No expansion into unrelated scope and no downgrade merely because it was found later. |
| Optional validation advice | A feasible options rationale recommends a regression for its key assumption, without executing it or changing scope. | The choice is judged on feasibility. The recommendation alone does not fail goal coverage or create a new worker round. |
| Small-change final report | A writer produces the change; the final report initially references the feature integration checkpoint. Finalize as small-change. | The effective report depends on the enabled checks; each check depends on the relevant producer. Re-finalizing preserves the graph. No report runs before its required results. |
| Late provider failure | The send boundary acknowledges success and clears a draft, but an incompatible provider rejects the image later. | Review identifies that the later error does not establish recoverable send failure. Check the acknowledged success, provider retrieval and final draft/attachment state separately. |
| Public options message | Run an options task in a Polish conversation; the technical rationale includes evidence paths, C1/C2 references and review history. | The chat displays a concise Polish explanation of the choice, benefit and tradeoff. Full technical fields remain available to the worker, reviewer and task details. |
| Long options history | Approve a selection with a long rationale, then reject and resume its consumer once. | The consumer receives the complete chosen variant and a dependency-reader reference. Full rationale and alternatives remain readable through MCP; they are not repeated automatically in the turn prompt. Current findings precede the goal and historical context; all acceptance criteria survive. |
| Role-specific completion | Start a writer, options worker, reviewer and planner through the native CLI transport. | Each footer names only result tools available in that role's tools/list. A rejected result remains correctable, stale-turn recovery uses current_turn, and only recorded=true ends a successfully recorded turn. |

For each case, record pass/fail/blocked for the requested outcome, requirement
coverage, dependency correctness, retained work, finding accuracy and unnecessary
worker/reviewer rounds. A raw number of findings or tasks is not a quality score.
For failures, keep the exact triggering input and relevant source/output evidence.
For an inconclusive case, name the missing observation rather than mark it passed.

When assessing compound acceptance criteria, trace the actual user entry point,
component transitions, success acknowledgement and final observable state. Evidence
that an error exists and evidence that some other error restores state are not
sufficient to establish that this particular failure is recoverable.

Prompt composition should keep authority and role constraints in the turn prefix,
current goal and acceptance in the task context, selected rigor in planning policy,
and recovery details in the applicable continuation. New recovery rules must not
replace the original task target or weaken an existing acceptance obligation.
