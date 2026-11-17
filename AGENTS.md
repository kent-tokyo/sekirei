# Sekirei agent instructions

## Current scope — 2026-11-16

The user reset all ongoing and pending improvement plans to reconsider why the engine is not getting stronger. No improvement experiment is currently selected. Do not resume an old plan, trial, training run, match, or autonomous improvement loop from a historical “next step.” This applies to local `ROADMAP.md`, `tasks/`, `docs/plans/`, design notes, and experiment records. See [the review](docs/plans/README.md). Preserve original verdicts, inputs, weights, and unused confirmation sets.

The long-term goal remains stronger Pure Safe Rust shogi play, ultimately surpassing engines such as Suisho and Hisui, within this Mac's resources. It is not authorization to restart the closed plans. New work follows the user's revised objective.

## Work and report

- Read [README.md](README.md), [the document index](DOCUMENTATION.md), and the relevant code before treating a capability as missing. The local roadmap holds current scope; dated records hold evidence.
- Within an authorized objective, continue useful bounded work autonomously. Rank evaluation, search, time use, opening play, and faster builds by expected strength benefit, evidence, and cost. Candidate counts and implementation counts are not success metrics.
- Before an expensive experiment, state the hypothesis, smallest useful check, and limits. Register its ID, source/binary/evaluator/input identities, decision rule, and stop rule before observing results. Reuse existing scripts and contracts.
- Use a small number of bounded sub-agents for independent work when useful. Assign a question, owned files, inputs, checks, and budget. One coordinator owns integration and resource admission; parallel review does not authorize parallel engine jobs.
- Verify artifacts with applicable tests or receipts. Keep implementation, correctness, speed, move quality, and strength conclusions separate. Add infrastructure or formal methods only for a concrete gap.
- Report concisely in Japanese: findings, uncertainty, and current state. During monitored long NNUE runs, report job state, completed pairs, registered statistic/verdict, and waits or termination at least every 30 minutes. Never imply background continuation without a verified job.

## Safety and resources

- Core engine logic is Pure Rust: no `unsafe` or external C++ wrappers. Preserve legal moves, repetition rules, board/history restoration, and integer NNUE do/undo equality.
- Keep the search-loop TT a fixed-size, lock-free atomic array without `Mutex` or `RwLock`; preserve coherent publication, generations, CAS, and valid scores.
- Bound speculative tasks, propagate parent cancellation, check it promptly, and release resources through RAII. Measure actual termination; dropping a handle is insufficient.
- Default to sequential, single-thread diagnostics and bounded training. Check active jobs, memory pressure, and disk capacity before heavy work. Fix time, threads, memory, output, and sample limits; do not silently extend them.
- When timing conditions fail, label any permitted fixed-node or exploratory result separately. It cannot qualify speed or strength or be pooled with strict measurements.
- Preserve failed evidence. A new recorded attempt after a verified technical fix or resource recovery may proceed within an authorized objective. Do not retry a statistical rejection to seek a pass.
- Stop when the objective is verified, the user stops or changes scope, required authorization/input is missing, the budget is reached, or no meaningful feasible action remains. Record the reason; old resumption conditions do not override the current reset.

## Experiments and adoption

- Freeze development inputs before selecting a candidate. Check integer correctness, move ranking, major errors, same-time cost, improvement/worsening counts, unresolved cases, and outlier sensitivity as relevant.
- Keep training, development, and independent confirmation separate. Known failures are debugging inputs, not fresh qualification. Lower validation loss alone is insufficient for a match candidate.
- New NNUE screens are preregistered and capped at 500 colour-reversed pairs. Freeze advancement/futility rules before play; use fresh-root confirmation only when warranted. Never extend, pool, or reinterpret completed trials.
- Adoption and publication require their own independent qualification. Existing search experiments retain independent H1 requirements. Exploration, screen advancement, confirmation, adoption, and publication are separate decisions; no unqualified candidate becomes default.
- Pin hardware, binary, evaluator/weight, settings, time, hash, threads, and corpus. Qualify 4T and 8T separately. Use a pinned YaneuraOu reference; distinguish NPS, useful work, same-time quality, and strength.
- Use material-only evaluation and Sekirei-generated openings for search tuning/SPSA and search-change screens. Suisho5 executables, weights, evaluations, and derived labels may only measure Sekirei against YaneuraOu, with separate evidence; never use them for training, candidate selection, constants/defaults, or public artifacts.
- Preserve M03's entry pass and `CONFIRMATION_NO_PASS` independently. Do not adopt M03, extend its completed match, or combine both stages into a new verdict.

## Data, Git, and verification

- Inspect Git status and relevant diffs; preserve unrelated work. Use an isolated checkout when qualification or publication requires reproducible inputs. Earlier release authorization does not authorize another release.
- Codex owns self-NNUE generation, training, weights, and candidate evaluation within authorized work. Preserve prior trials and hold-outs. Keep learning data and private/unqualified weights out of GitHub.
- Qualified weight publication needs separate authorization, license, hashes, and evidence. Archive retained cold data under `/Volumes/ESD-EPA/Projects/sekirei_data`; verify archive/restored hashes and restoration steps before deleting local copies. Delete only verified-unused regenerable caches after checking active users and references.
- Follow [development checks](docs/development.md) for the change. Local tests, hosted CI, benchmarks, registry publication, and deployment are separate checks. The historical 64-core contention target (<5%) and 10,000,000 random-validation target require their own evidence, not a routine rerun.
- Prefer capabilities already shipped by `lineprior`, `shogiesa-core`, or `veridict` over duplicate implementations.
- Keep current instructions short. Preserve superseded instructions in [history](docs/agent-instructions/README.md) and detailed outcomes in their evidence files. Do not edit frozen artifacts to make old results fit new policy.
