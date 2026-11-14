# Agent instruction history

The repository-root [AGENTS.md](../../AGENTS.md) contains the consolidated current instructions. This directory preserves their history; it is not a second set of tasks to execute. Consult only the portion relevant to a question about prior authorization, trial scope, or specialist writing guidance.

## 2026-11-12: separate current policy from historical requests

The user requested improvement of AGENTS.md after discussing outcome-based instructions, resource budgets, and excessive process overhead.

[AGENTS-before-2026-11-12-refresh.md](AGENTS-before-2026-11-12-refresh.md) is a byte-for-byte copy of the working file immediately before this reorganization, including its existing uncommitted additions.

- Size: 20,187 bytes; 193 lines.
- SHA-256: `8a95e827a7035b2205ebd1276a894caf176c039c5a6e24c30f0abb237b093fdc`.
- It retains the original architectural vision, role descriptions, development phases, validation targets, dated operational instructions, release requests, and newsletter/property-market writing guidance.

The reorganization makes these distinctions explicit:

- The 2026-10-26 recovery instruction supersedes the blanket automatic-retry prohibition for new attempts after a verified fix or resource recovery. It does not authorize replaying rejected statistical trials or changing a frozen continuation contract.
- The 2026-11-11 NNUE development policy permits exploration without H1 or improvement in every metric. It does not remove independent qualification for adoption, revise existing search experiments' H1 requirements, or alter old verdicts.
- Completed requests for ten candidates, particular releases, and M03's confirmation remain records of those tasks, not instructions to repeat them.
- Long-term architectural and scale targets remain qualification goals. Earlier foundation phases do not imply that their already-implemented components should be rewritten.
- Newsletter and property-market rules remain available in the final section of the archived file for those writing tasks. They are omitted from the routine engine-development context.

No historical instruction or trial artifact was deleted. The current file changes how work is directed and reported; it does not change engine behavior, trial outcomes, or publication state.

## 2026-11-12: build on checked results with bounded delegation

The user requested incorporating the lessons discussed from the [November 11 Zaikei article](https://www.zaikei.co.jp/article/20261111/877067.html). The [primary technical report](https://claude.dev/blog/safely-scaling-multi-agent-teams-with-fable-5-2/) describes agents dividing a formalization into dependent tasks, sharing completed work, and checking proofs with Lean. It also reports resource exhaustion on a shared machine. These are the authors' experimental results, not measurements of Sekirei.

The current AGENTS.md applies the following lessons to this project's constraints:

- Decompose nontrivial work by useful question, dependency, artifact, and completion condition; retain both checked results and failed evidence for the next decision.
- Delegate independent work to a small number of agents with explicit ownership and budgets. One coordinator controls integration and shared compute; review concurrency does not expand training or engine-run budgets.
- Verify artifacts with the applicable tests, validators, or receipts, and inspect whether those checks establish the intended property. An agent's completion report alone is insufficient.
- Treat executable invariants, formal proofs, performance measurements, and independent strength confirmation as distinct evidence. Consider formal methods only for a concrete correctness gap, including the correspondence between a model and the actual Rust code.

The local ROADMAP.md applies these rules to the pending quiet-position selection diagnostic, assigning implementation, exclusion review, and result analysis as roles without requiring separate agents for every role. Existing Git, plans, and evidence files provide the coordination record. This update introduces no database, mandatory proof framework, new experiment, changed trial verdict, or publication authorization. Registered plans and historical instruction snapshots remain unchanged.
