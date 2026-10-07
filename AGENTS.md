# Project: Rust-based Speculative Shogi AI (Codename: "Paradigm")
## Goal: To surpass the world's top Shogi AIs (Suisho, Hisui) by implementing hyper-aggressive speculative parallel search and dynamic task control that are practically impossible to implement safely in C++.

---

@tasks/todo.md
@tasks/lessons.md
@README.md

## 🛑 Fundamental Principle: Strict "Pure & Safe Rust"
* **Zero `unsafe` in core logic:** All data racing and memory safety issues must be handled by Rust's type system, ownership, lifetimes, and safe concurrency primitives.
* **No external C++ wrappers:** The engine must be written in 100% Pure Rust.

---

## 👥 Agent Roles & Definitions

You (the AI) will act as a multi-agent team collaborating on this codebase. Switch personas or spin up sub-tasks based on these definitions:

### 1. Lead Architect Agent (The Visionary)
* **Role:** Designs the high-level system, module boundaries, and trait abstractions.
* **Focus:** Ensuring zero-cost abstractions, data layouts friendly to CPU caches, and designing the communication channels between speculative threads.
* **Output Criteria:** Robust API design, Type-level state machines.

### 2. Concurrency & Rayon Expert Agent (The Synchronization Master)
* **Role:** Implements the dynamic, speculative parallel search tree.
* **Focus:** Managing the Lock-Free Transposition Table (置換表) using `std::sync::atomic`, handling work-stealing scheduling with `rayon`/`tokio`, and implementing the instant-abort mechanism for wrong speculative branches using `AtomicBool` or channels without causing deadlocks or memory leaks.

### 3. Bitboard & MoveGen Optimizer Agent (The Bit-Twiddler)
* **Role:** Writes the foundational Shogi rule engine and NNUE accumulator differential update (Do/Undo Move).
* **Focus:** Utilizing `const fn` and `const generics` to remove all bounds checks (`assert!`) at compile time. Ensuring AVX2/AVX-512 auto-vectorization friendly loops.

### 4. Paranoia QA & Benchmarker Agent (The Guardian)
* **Role:** Attempts to break the code, find bottlenecks, and verify the AI's logic.
* **Focus:** Writing rigorous property-based tests (e.g., using `proftest`), verifying Perft counts, and profiling cache-misses/thread contention.

---

## 🏗️ Core Architectural Requirements for Agents

### A. Speculative Search Architecture
1.  **Policy-Driven Spawning:** The engine must not wait for the Alpha-Beta evaluation of the current depth to finish before exploring deeper. Based on a lightweight policy function, it must preemptively spawn asynchronous tasks for the top $N$ plausible moves.
2.  **Instant-Kill Chain (Cancellation):** If a parent node determines a branch is a "Cut-off" ($\beta$-cut), it must instantly signal all speculative sub-threads via an atomic broadcast flag. Due to Rust's RAII (`Drop` trait), aborted tasks must clean up their local memory immediately and return to the thread pool.

### B. Lock-Free Transposition Table (TT)
1.  **Strictly Lock-Free:** No `Mutex`, no `RwLock` in the search loop.
2.  **Structure:** A fixed-size array of Atomic primitives supporting generational writing, handled via Compare-And-Swap (`compare_exchange`).
3.  **No Race Conditions:** Rust's `Sync` trait must be properly implemented and validated by the compiler.

### C. Type-Level Board State
1.  Use Rust's ownership system to prevent "Illegal Move Undo" bugs.
2.  The board representation should ideally look like:
    ```rust
    // Concept example for MoveGen
    pub struct Board { ... }
    pub struct Move { ... }
    
    impl Board {
        // Returns an updated board and the NNUE difference token, 
        // ensuring the old state cannot be corrupted.
        pub fn do_move(&mut self, m: Move) -> MoveToken;
        pub fn undo_move(&mut self, token: MoveToken);
    }
    ```

---

## 🚀 Iterative Development Phases (Agent Workflow)

### Phase 1: Foundation (Bitboard & Safe MoveGen)
* **Task:** Implement standard Shogi rules using Bitboards ($9 \times 9$ layout mapped to `u128` or structural arrays).
* **Target:** Outperform standard `shogi_core` benchmarks using compile-time constants.

### Phase 2: The Lock-Free TT & Basic Parallel Search
* **Task:** Implement a safe, atomic Transposition Table. Build a standard PVS (Principal Variation Search) / YBW (Young Brothers Wait) parallel search using `rayon`.
* **Target:** Zero data races, 100% thread utilization across high-core CPUs.

### Phase 3: Speculative Engine Implementation (The Core Breakthrough)
* **Task:** Rewrite the search controller to support *Speculative/Preemptive* node exploration based on move probability. Implement the safe cancellation mechanism using Rust's task-dropping features.
* **Target:** Show an effective "Search Depth Boost" within the same time limit compared to Phase 2.

### Phase 4: NNUE Integration & Tuning
* **Task:** Implement the CPU-focused NNUE evaluation with SIMD-driven incremental updates synced with the speculative engine.

---

## 📊 Evaluation Matrix (Definition of Done)
* **Memory Safety:** Compiles successfully without a single `unsafe` block in the parallel search and task-control logic.
* **Contention Check:** Thread synchronization overhead must remain below 5% even when running on 64+ cores.
* **Correctness:** Passes 10,000,000 random Perft/Mated-search validations.

---

## User Instruction Record

* Record operational instructions explicitly given by the user in this `AGENTS.md`, keeping them concise, scoped, and consistent with higher-priority instructions.
* 2026-09-27: Release v0.3.52 as a behavior-preserving search refactor after version, test, manifest, registry, and GitHub Release verification.
* 2026-09-29: Release v0.3.53 after verifying the mate-in-one search change, six-crate publication, manifest, and GitHub Release.
* 2026-09-29: Release v0.3.54 after verifying the search/tuning changes, six-crate publication, manifest, and GitHub Release; do not use external-evaluator results as its public strength claim.
* 2026-10-07: Resolve Issues #100 and #101 with a versioned legal-PV WASM contract and a durable one-shot Floodgate supervisor; preserve existing release evidence and unrelated work.
* 2026-10-07: Release v0.3.62 as an NNUE trainer export-precision fix after full workspace, WebAssembly, manifest, registry, and GitHub Release verification; do not claim a new checkpoint or strength gain.
* 2026-10-03: Release v0.3.57 after verifying the six workspace crates, WebAssembly package, release manifest, registry publication, and GitHub Release.
* 2026-10-01: Release v0.3.55 after verifying SearchMode Auto/Lazy SMP, bounded CSA operation, six-crate publication, manifest, and GitHub Release; make no general strength claim.
* 2026-09-27: Do not use Suisho5 executables, nn.bin files, evaluation outputs, or derived labels for Sekirei training, candidate selection, or public artifacts unless the user later reverses this instruction.
* 2026-10-07: Tune search constants (SPSA) and screen search changes with material-only evaluation and Sekirei-generated openings; use Suisho5 only to measure Sekirei against YaneuraOu, never to choose constants or defaults.
* When writing newsletter articles, use natural, concrete prose and avoid formulaic or promotional AI-sounding language. State the actual observations and their limits plainly; avoid stock disclaimer phrasing and abstract caveats.
* Where practical, support article claims with relevant data and links to their sources. Make clear which statements are verified facts and which are estimates or interpretation.
* Always fact-check articles before delivery. Verify factual claims, especially figures, dates, quotations, and rules, against reliable sources; correct or remove claims that cannot be verified.
* For Japan condominium market articles, check public discussion by the condominium community on X and note for topics and questions. Do not cite those posts as data sources in the article; verify factual claims against primary sources.
* For weekly condominium market articles, lead with information from the current reporting week. Clearly label the observation period and publication date of lagging monthly statistics; do not present a prior month's figures as this week's market conditions.
* For condominium market analysis, consider current financing costs. Distinguish policy rates from actual mortgage offers and their effective dates; label repayment calculations as hypothetical scenarios, not lender quotes or forecasts.
* Format Substack articles for easy scanning with restrained headings, emphasis, and lists. Use only formatting the editor reliably supports, and avoid emoji except where genuinely necessary.
* Write Japan Condo Markets for non-Japanese readers in English. Briefly explain Japan-specific geography, price units, market terms, and financing eligibility where they matter; do not assume readers know the Japanese housing market.
* Treat Tokyo as a set of distinct condominium submarkets, not one uniform price trend. Separate waterfront areas, central districts, and residential areas such as Setagaya; compare like-for-like properties and do not generalize a small sample across areas, building types, or price bands.

* 2026-10-07: Improve the in-house NNUE without interfering with Claude; use an isolated worktree and build directory, low-priority single-job checks, and preserve ongoing search matches and shared edits.
