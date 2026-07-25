# Phase A2 seeded-init (B1/B2/B3) audit — read-only, manifests-only

Status: **audit complete (2026-07-25, second pass)**. Every value below is
either transcribed from files already on disk
(`data/runs/phaseA2_20260724/checkpoints_b{1,2,3}/*.meta.json`, `*.log`, and
`results/phase_a2/b1_vs_a/{SUSPENDED.md,state.json}`) or derived by read-only
I/O against those same files' bytes (`shasum -a 256`, and a new pure-Python
parser, `scripts/audit_nnue_weight_stats.py`, that mirrors
`crates/sekirei-core/src/nnue.rs`'s `read_weights` byte layout exactly).
**No cargo/rustc was invoked, nothing was retrained, no teacher labels were
regenerated, no gate was re-run** — this pass closed the fields the first
pass (below) had listed as preflight items, using only file hashing and byte
parsing, deliberately avoiding any compile step because swap was at ~90% (up
from ~52% at the first pass) with another project's job pinning a CPU core
at the time. This is a companion to `phase_a2_seeded_init_preregistration.md`
(`af5d6d4`) — that pre-registration is frozen and not edited here.

## What B1/B2/B3 are

Per the pre-registration: same recipe as v011, only `init_seed` varies (B1=42, B2=43, B3=44); `split_seed=42` and `shuffle_seed=null` are held identical across all three by design, so any difference between them isolates the effect of initialization seed alone. **B1 (seed 42) is the pre-registered primary candidate**, fixed before any of the three were trained specifically so the result couldn't retroactively cherry-pick a "winning" seed. B2/B3 exist only as a reproducibility/sensitivity check — **this audit does not propose swapping either in as a candidate**, per today's explicit constraint.

Disclosed pre-registered deviation: all three train against one shared, freshly generated teacher-label cache (current engine, post-Sprint-1 search) — v011's original labels no longer exist to compare against directly.

## Per-field audit table

Three states, not two — "present but null" (a schema field exists, the pipeline never computed a value for it) is a different finding from "absent entirely" (no such key was ever written), and both are different from a real value:

| Field | B1 (seed42) | B2 (seed43) | B3 (seed44) | State |
|---|---|---|---|---|
| `weight_path` | `checkpoints_b1/weights_b1_seed42.bin` (final); `.epoch{1,2,3}.bin` + `.best.bin` also present | `checkpoints_b2/weights_b2_seed43.bin` (+ same per-epoch/best set) | `checkpoints_b3/weights_b3_seed44.bin` (+ same set) | present (directory listing; no literal `weight_path` JSON key in any meta file) |
| `sha256` | `019d13f284447b6afc3905dfccb7a5a570e4e3d3b08655a7f3a7b43b174a1385` | `e696bde320c1f0e3f7e881a6166eb10c65f44a19df4b95eb90734cc1490cced6` | `a3680b18065d6c9c33b80e3a5d1ec58f40986df1881d900686a02c44b3946b89` | **now present for all three** (`shasum -a 256`, this pass). B1's value matches `SUSPENDED.md`'s manifest exactly (cross-check passed). Each of `.bin`/`.best.bin`/`.epoch3.bin` within one seed is byte-identical (same sha256), confirming best==epoch3==final at the byte level, not just by log text. |
| `file_size` | 1,305,356 bytes | 1,305,356 bytes | 1,305,356 bytes | present — matches the format's expected size exactly (`8 + INPUT·L1·2 + L1·2 + 2·L1·L2·4 + L2·4 + L2·4 + 4` = 1,305,356 for `INPUT=2420 L1=256 L2=32`) |
| `checkpoint_hash` (short, not sha256 — a distinct field actually present in every `epoch3.meta.json`) | `bc1df26edc2002de` | `88fa2b7baa0c3da6` | `ba3805f7b41552b1` | present with value (all three) |
| `dataset_hash` | `10479297805450667706` | same | same | present with value, identical across all three (expected — shared teacher cache) |
| `split_hash` | `3527444922185093112` | same | same | present with value, identical across all three (expected — `split_seed=42` fixed by design) |
| `teacher_cache_fingerprint` | `c1fa80d562f54d11d7795553e7eecbf932fd7ce22841ccce671bc0bbd142c916` (sha256 of `data/runs/phaseA2_20260724/teacher_cache.jsonl`) | same | same | **resolved this pass**. Still no literal field in any meta.json, but `train_b1.log` shows B1 generating and writing this exact file ("teacher cache: 10121 new entries computed" / "teacher cache written → ...teacher_cache.jsonl"), while `train_b2.log`/`train_b3.log` both show "10121 entries loaded from ...teacher_cache.jsonl" / "all 10121 entries from cache (no search)" — literal reuse of the identical file, confirmed by hashing it once and citing the shared path all three logs reference. This closes the field the first audit pass left unresolved. |
| `architecture` | `"INPUT=2420 L1=256 L2=32"` | same | same | present with value |
| `init_seed` | `42` | `43` | `44` | present with value (the one deliberately-varied field) |
| `split_seed` | `42` | `42` | `42` | present with value, identical by design |
| `shuffle_seed` | `null` | `null` | `null` | present but null (field exists, value intentionally unset per pre-registration — not a gap) |
| `epochs` | `3` | `3` | `3` | present with value |
| `selected_epoch` | — | — | — | **absent entirely** as a literal field; inferable from `train_b*.log`'s `best (valid_loss=…) → …best.bin (from …epoch3.bin)` line — best == epoch3 == final for all three |
| `lr_schedule` | `"StepHalf"` | same | same | present with value |
| `wdl_lambda` | `null` | `null` | `null` | **present but null** for all three epochs, all three arms |
| `valid_cp_mse` | `null` (literal JSON null) | `null` | `null` | **genuinely unknown, not softened**: the CP/WDL-decomposed loss this field name implies was never computed for this run. A related-but-different quantity *is* available and should not be confused with it: the aggregate `valid: loss_raw=…`/`loss_weighted=…` line in `train_b*.log` (B1=209449.95, B2=210353.60, B3=211685.39 at epoch3) is a real, present validation loss value — just not split into CP-MSE vs. WDL-loss components. Report the raw number as present, the decomposition as unknown. |
| `valid_wdl_loss` | `null` | `null` | `null` | present but null, all three |
| `ft_dead_neurons` | `0` | `0` | `0` | present with value, identical |
| `l2_dead_neurons` | `12` | `12` | `16` | present with value — **B3 differs from B1/B2** (12 vs. 12 vs. 16); the one metric where seed variation visibly shows up in this table |
| `selected_epoch` (derived, not a literal field) | `3` | `3` | `3` | still no literal field, but now doubly confirmed: `train_b*.log`'s "best (valid_loss=…) → …best.bin (from …epoch3.bin)" line, **and** this pass's sha256 showing `.bin`/`.best.bin`/`.epoch3.bin` are byte-identical within each seed. Derived-with-evidence, not a guess, not `unknown`. |
| `weight_variance` (FT / L2 / out, computed via `scripts/audit_nnue_weight_stats.py`, this pass) | ft=39.53 l2=0.0702 out=2.673 | ft=40.51 l2=0.0763 out=3.032 | ft=39.42 l2=0.0619 out=2.639 | **resolved this pass**. All three: nonzero variance in every layer (`zero_init_collapsed_suspected: false`) — seeded init worked, no collapse, for all three seeds, with broadly similar magnitudes across seeds (a reproducibility sanity signal, not proof of equal strength). Cross-check: the same script run against `data/weights_v011_opening_combined.bin` (A) reports `l2_variance=0.0` and `out_variance=0.0` exactly — independently reproducing `docs/weights_registry.toml`'s existing `zero_init_collapsed=true` finding for A, using a from-scratch script rather than trusting the prior audit's number. |
| `weight loader can read it` (`loader_would_accept`, this pass) | true | true | true | Confirmed for all three (and for A) via a Python re-implementation of `nnue::read_weights`'s exact checks (magic ∈ {SEKIRW01, JANOSW03}, byte length == 1,305,356) — **not a literal invocation of the compiled Rust loader**; that would require `cargo run`/`cargo test`, deferred given the swap situation described above. This substitutes a format-conformance check for a binary-execution check; flagged explicitly rather than silently treated as equivalent. Re-run the real loader (e.g. via the new `crates/sekirei-core/examples/repro_multiweight_onelock.rs` or a plain `load_weights` call) once compute is available, to fully close this item. |
| `B1 remains primary seed` | yes | n/a | n/a | Unchanged from the pre-registration: B1 (seed42) was fixed as primary before any of the three were trained, specifically so no result here could retroactively promote B2/B3. This audit pass does not propose otherwise — see "What this audit does not do" below. |

## B1-vs-A gate: zero games played, despite the directory name

`results/phase_a2/b1_vs_a/` implies a result exists. **It does not.** Per `SUSPENDED.md` and `state.json`, read directly:
- **Games launched: 0.** All 1700/1700 shards are `"status": "pending"`. `confirmed_prefix: 0`. `decisive_verdict: null`.
- No shard ever advanced past its initial paused state — the resource monitor held every launch back from the first loop iteration, first on load average (18.3), then on swap (~85.7%, steady, never dropping below the default 50% pause threshold).
- No `combined.json`/`combined.jsonl` exists (only written once at least one shard's result is confirmed).
- Suspension was an explicit **user decision**, not a crash, not an auto-stop from hitting a verdict, and no background auto-resume is active.
- Manifest values are fully recorded for whenever this resumes: B1 sha256 `019d13f284447b6afc3905dfccb7a5a570e4e3d3b08655a7f3a7b43b174a1385`, A (`data/weights_v011_opening_combined.bin`) sha256 `a45be6099c0936283e79f34d380a4dbc7ba681796bb0bb56b2cd743c2c786ea6`, corpus `data/gate/openings_gateB.sfen` (1707 positions, sha256 `816fdf7661989b348bf1c2e078fd6b5748ff9cfc14fa0aed3b83c6df39d56545`), SPRT bounds elo0=0/elo1=20/α=0.05/β=0.05, trinomial paired-by-id.

**Say this in plain words for anyone reading only the directory name: zero evidence currently exists about whether B1 beats A. Nothing today's audit found changes that.**

## Preflight items — status after this pass

1. ~~sha256 for B2 and B3's final weight files~~ — **done this pass** (see table above).
2. ~~`weight_variance` for all three arms~~ — **done this pass**, via the new `scripts/audit_nnue_weight_stats.py` (see table above).
3. **`teacher_cache_fingerprint`** — **resolved as a derived value this pass** (sha256 of the shared `teacher_cache.jsonl`, cross-referenced against all three logs). Still not a literal field the training pipeline writes to `meta.json` — promoting it to an explicit output field remains a real, open, low-priority improvement for future runs, but it's no longer a gap in *this* audit.
4. **Resolve the swap-pause threshold before resuming `b1_vs_a`** — still open, not resolved by this pass (out of scope for an audit; it's a decision for whoever launches the gate). Steady ~85.7% swap never cleared under the default `--max-swap-pct 50` last attempt; a ~92% figure was discussed but never applied. Swap is *worse* now (~90%, this session) than at the last attempt — re-launching with the same default will almost certainly re-enter the identical paused state. See the preflight doc (`docs/experiments/phase_a2_b1_vs_a_gate_preflight.md`) for this decision surfaced explicitly, not resolved on the user's behalf.
5. **Add B1/B2/B3 entries to `docs/weights_registry.toml`** — still open. sha256 now exists (item 1 is done), so this is unblocked, but adding registry entries is a docs edit beyond this audit's scope (Task 1 was "complete the audit," not "extend the registry") — left for a future pass.
6. **Rebuild and re-hash `target/release/sekirei`/`target/release/sekirei-match` before actually launching the gate** — new item, found while preparing the gate preflight: `SUSPENDED.md`'s recorded binary hashes are pinned to git commit `af5d6d4`; `HEAD` has since advanced (three commits landed earlier this session, all test/docs files, no engine source changes) — but that a rebuild would reproduce an identical binary is an inference, not something verified without actually building. See the preflight doc.

## What this audit does not do

Does not propose B2 or B3 as an alternative candidate to B1. Does not run,
resume, or estimate the outcome of the `b1_vs_a` gate. Does not retrain,
regenerate teacher labels, or invoke cargo/rustc — every value added in this
pass came from `shasum`/file reads and a new pure-Python byte parser, not a
compile. Does not modify `phase_a2_seeded_init_preregistration.md`, any
weight file, or `docs/weights_registry.toml` (items 5–6 above remain open,
deliberately not done as part of this audit).
