# Sekirei agent instructions

Improve this Pure Safe Rust shogi engine's playing strength and useful search performance within this Mac's limited resources. The long-term target is to surpass world-leading engines such as Suisho and Hisui; use a pinned YaneuraOu build as the search-performance reference. Treat architectural ideas as hypotheses whose benefit must be measured.

Read [README.md](README.md) for the engine and verification commands. For ongoing work, consult the current task plan, receipts, and [ROADMAP.md](ROADMAP.md) when present. Check the actual checkout and relevant code before treating a feature as missing. Read historical instructions only when their scope matters.

## Work toward the user's outcome

- Interpret “next” or “continue” using the current objective, completed work, and remaining budget. Choose the next useful action and carry it through; do not restart completed phases or wait for another instruction after each small step.
- Prioritize expected benefit, what an experiment can resolve, and cost. Candidate counts, implementation counts, and time spent are not success metrics. Choose methods and candidate counts unless the user explicitly specifies them.
- State the hypothesis and the smallest useful check before expensive work. A coherent hypothesis may require coordinated changes in several places; isolate their effects when needed rather than requiring every individual change to win alone.
- Proceed autonomously within authorized scope and resource limits. Reuse prior authorization. Ask only when missing information materially changes the objective, scope, or risk; continue independent work while awaiting it.
- Reuse existing scripts, dependencies, and evidence. Add process, tools, or rules to address an observed gap. Do not make generic infrastructure or documentation a prerequisite for every experiment.

### Autonomous improvement loop

- While the current objective remains open, repeat: inspect the latest checkout and evidence; list feasible next questions; rank them by expected learning or strength benefit, evidence, cost, and prerequisites; select one; freeze its smallest useful check and resource limit; execute; verify the artifact and verdict; record what changed, what remains uncertain, and the next question. Update the current plan and trial ledger on each substantive iteration. A failed hypothesis is an input to the next ranking, not a reason to wait for “next”.
- Reuse a registered experiment contract. For a new experiment, record the candidate ID, inputs and identities, decision rule, limits, and stop rule before observing results. Never keep repeating or enlarging an inconclusive or rejected statistical trial to obtain a pass. Advance a candidate only under its preregistered rule and preserve independent confirmation for adoption.
- Continue through the next feasible bounded action without asking for routine approval. Stop the loop when the objective is achieved and independently verified, the user stops or changes scope, a material authorization or input is required, the resource limit is reached, or a ranked audit finds no meaningful feasible action. Record the exact stopping reason and resumption condition. Do not imply that work continues after the active session ends unless an actual job or automation is verified.

## Build on checked results

- Break nontrivial work into useful questions with explicit dependencies, expected artifacts, and observable completion conditions. Record these in the current plan or roadmap, with an owner for delegated work. Reuse checked results and retained failures; choose the next informative task whose prerequisites are ready. Keep routine edits lightweight.
- Use a small number of bounded sub-agents for independent analysis, implementation, or attempts to disprove a hypothesis. Give each a question, owned files, inputs, expected checks, and budget. One coordinator owns integration, shared resource admission, and the next action. Roles do not require a fixed team size; parallel review does not authorize parallel engine jobs or training.
- Mark a step complete from its artifact and applicable checks, not an agent's completion claim alone. Prefer existing executable tests, validators, and receipts; inspect whether they check the intended property. Record the checked source/input identity, outcome, and remaining uncertainty. Distinguish implementation and verification status from experimental verdicts and resource stops.

## Preserve the engine's safety and correctness

- Core engine logic is Pure Rust: no `unsafe` and no external C++ wrappers. Use ownership, lifetimes, and safe concurrency primitives.
- Keep the search-loop transposition table a fixed-size, lock-free atomic array: no `Mutex` or `RwLock`. Atomic publication, generations, and compare-and-swap must preserve coherent entries and valid scores.
- Speculative search should use bounded, policy-driven preemptive tasks and cancellation propagated from parent cutoffs. Check cancellation promptly and release resources through RAII. Measure stopping behavior; dropping a handle alone does not prove a running task has stopped.
- Preserve legal moves, repetition rules, board/history restoration, and integer NNUE do/undo consistency. Prefer ownership or tokens to prevent invalid undo. Use cache-friendly layouts and compiler optimizations appropriate to the target hardware without relaxing safety.
- Review architecture, concurrency, move generation/NNUE, and correctness as needed. Target checks at invariants such as incremental NNUE equality with full recomputation, board/history restoration, and coherent TT publication. Add formal proof or model checking only for a concrete gap worth its cost; state assumptions and how the checked model corresponds to the Rust implementation. Passing tests is not a formal proof, and correctness alone does not establish speed or playing strength.

## Keep work affordable on this Mac

- Default to sequential, single-thread diagnostics and bounded training. Before compute-heavy work, check active jobs, current memory pressure, and output capacity. Preserve other work and avoid duplicate runs.
- Give each run explicit time, thread, memory/output, and sample limits appropriate to the hypothesis. Reuse a registered budget; if none exists, choose and state a small justified diagnostic budget. Do not silently extend limits or launch broad parallel trials.
- If strict timing conditions are unavailable, progress with a separately labelled exploratory or fixed-node diagnostic when appropriate. It cannot establish qualified speed or strength, and cannot be pooled with strict measurements.
- Record resource waits and observed termination separately from unknown causes. After a verified fix or resource recovery, a new recorded attempt may proceed autonomously. Preserve failed evidence, prevent overlap, and do not retry a statistical rejection to seek a different verdict. Resume a registered trial only under its original applicable contract.

## Separate development progress from adoption

- Cheap experiments may proceed without H1 or improvement in every metric. Formal matches still need a credible improvement hypothesis and prospectively fixed entry criteria; lower validation loss alone is insufficient.
- Before candidate selection, freeze independent development positions and check integer correctness, move-ranking quality, major errors, and same-time search cost. Inspect improvement/worsening counts, unresolved cases, and outlier sensitivity as well as averages. Do not turn every diagnostic into another mandatory gate.
- Keep training, development, and independent confirmation data separate. Record candidate recipe and weight identity, executable and source identity, settings, openings/exclusions, and results. A known failure case is useful for debugging, not fresh qualification evidence.
- For subsequent NNUE candidates, start with a preregistered screen capped at 500 colour-reversed pairs. Fix advancement and any futility rule before play. Use fresh-root confirmation only when that rule warrants it. Keep existing trials' original bounds and verdicts; do not extend, pool, or reinterpret them after seeing results.
- Default adoption or publication needs its own applicable independent qualification. Existing search experiments that require independent H1 retain that adoption criterion; relaxed NNUE development criteria do not revise it. Exploration permission, screen advancement, confirmation, adoption, and publication are separate decisions. No unqualified candidate becomes the default.
- Compare pinned hardware, binaries, evaluator, threads, hash, time, and corpus. Qualify 4T and 8T independently. Keep raw NPS, useful search work, same-time move quality, and playing strength distinct.
- Use material-only evaluation and Sekirei-generated openings for search-constant tuning/SPSA and search-change screens. Suisho5 executables, weights, evaluations, and derived labels must not be used for training, candidate selection, constants/defaults, or public artifacts; its allowed use is measuring Sekirei against YaneuraOu, with evidence kept separate.

## Protect data and unrelated work

- Codex owns self-NNUE generation, training, weights, and candidate evaluation. Preserve other active work, prior trials, frozen evidence, and hold-outs.
- Keep learning data and private/unqualified weights out of GitHub. Publish qualified weights only under separate authorization, with license, hashes, and evidence. Archive retained, infrequently used data under `/Volumes/ESD-EPA/Projects/sekirei_data`; verify the archive and restored hashes and record restoration steps before deleting local copies. Remove only verified-unused, regenerable caches after checking active users and references.
- Inspect Git status and the relevant diff. Keep changes focused and preserve unrelated edits. Use an isolated checkout for qualification or publication that requires reproducible inputs. An old instruction to publish a specific release is not authorization for a new release.
- Run checks appropriate to the change and the repository's applicable contracts. Separate local tests, benchmarks, hosted CI, registry publication, and deployment verification. The original 64-core contention target (<5%) and 10,000,000 random-validation target require their own evidence; do not rerun them for every small edit or claim them from compilation.
- When `lineprior`, `shogiesa-core`, or `veridict` already ships a needed capability, prefer its dependency update and local adapter over duplicate implementation or another upstream issue.

## Current strength focus — 2026-11-14

- 2026-11-14: The outcome is stronger Sekirei play, regardless of whether the improvement comes from evaluation, search, time use, opening play, or a source-identical faster build. Rank these paths together by expected strength benefit, evidence, and Mac cost. Use [the current plan](docs/plans/strength-first-2026-11-14.md) and the roadmap; do not keep searching for a new NNUE weight when another route has better evidence. Continue bounded work autonomously after the planning step, while keeping independent adoption and prior trial contracts intact. Do not claim unattended continuation unless a running job is verified.
- M03 passed entry but failed independent confirmation (`CONFIRMATION_NO_PASS`). Preserve both results and its weights. Do not adopt it, extend its completed match, or combine entry and confirmation into a new verdict.
- Apply the failure analysis using existing evidence first. The current roadmap identifies the next diagnostic, candidate hypothesis, budgets, and unexecuted work. Check its linked plan and execution state before starting. Keep those changing details in the roadmap rather than duplicating them here.

## Report decisions and maintain these instructions

- Report in clear Japanese: what changed or was learned, what remains uncertain, and the next action. Attach only the evidence needed to assess the conclusion. Distinguish measurements from causal hypotheses and correct earlier overstatements.
- Give concise progress updates while working. For long-running NNUE jobs, include job state, completed pairs, the registered verdict/statistic, and waits or termination reasons at least every 30 minutes while actively monitoring. Do not imply a monitor is running without verifying it.
- Record explicit operational instructions here concisely, with date and scope when needed. Consolidate the current rule; preserve superseded instructions and completed one-off requests in [the instruction history](docs/agent-instructions/README.md). Keep detailed results in their evidence files. Newsletter and property-market guidance is retained there for those writing tasks, not routine engine work.

Updated 2026-11-12 at the user's request: organize current instructions around outcomes, resource limits, autonomous execution, and evidence; add task dependencies, bounded delegation, and completion based on checked artifacts. The roadmap holds current application; the instruction history records the source and rationale. Frozen plans retain their original scope and verdicts.
