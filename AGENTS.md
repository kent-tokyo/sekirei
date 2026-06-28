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