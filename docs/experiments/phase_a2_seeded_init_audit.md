# Phase A2 seeded-init (B1/B2/B3) audit — read-only, manifests-only

Status: **audit only, no recompute**. Every value below is transcribed from files already on disk (`data/runs/phaseA2_20260724/checkpoints_b{1,2,3}/*.meta.json`, `*.log`, and `results/phase_a2/b1_vs_a/{SUSPENDED.md,state.json}`), read on 2026-07-25. Nothing was retrained, re-hashed, or re-gated to produce this doc. This is a companion to `phase_a2_seeded_init_preregistration.md` (`af5d6d4`) — that pre-registration is frozen and not edited here.

## What B1/B2/B3 are

Per the pre-registration: same recipe as v011, only `init_seed` varies (B1=42, B2=43, B3=44); `split_seed=42` and `shuffle_seed=null` are held identical across all three by design, so any difference between them isolates the effect of initialization seed alone. **B1 (seed 42) is the pre-registered primary candidate**, fixed before any of the three were trained specifically so the result couldn't retroactively cherry-pick a "winning" seed. B2/B3 exist only as a reproducibility/sensitivity check — **this audit does not propose swapping either in as a candidate**, per today's explicit constraint.

Disclosed pre-registered deviation: all three train against one shared, freshly generated teacher-label cache (current engine, post-Sprint-1 search) — v011's original labels no longer exist to compare against directly.

## Per-field audit table

Three states, not two — "present but null" (a schema field exists, the pipeline never computed a value for it) is a different finding from "absent entirely" (no such key was ever written), and both are different from a real value:

| Field | B1 (seed42) | B2 (seed43) | B3 (seed44) | State |
|---|---|---|---|---|
| `weight_path` | `checkpoints_b1/weights_b1_seed42.bin` (final); `.epoch{1,2,3}.bin` + `.best.bin` also present | `checkpoints_b2/weights_b2_seed43.bin` (+ same per-epoch/best set) | `checkpoints_b3/weights_b3_seed44.bin` (+ same set) | present (directory listing; no literal `weight_path` JSON key in any meta file) |
| `sha256` | `019d13f284447b6afc3905dfccb7a5a570e4e3d3b08655a7f3a7b43b174a1385` (`results/phase_a2/b1_vs_a/SUSPENDED.md`, manifest table) | — | — | **present for B1 only**; absent for B2/B3 (no sha256 has been computed for either — computing one, even cheaply, is out of scope for a manifests-only audit) |
| `checkpoint_hash` (short, not sha256 — a distinct field actually present in every `epoch3.meta.json`) | `bc1df26edc2002de` | `88fa2b7baa0c3da6` | `ba3805f7b41552b1` | present with value (all three) |
| `dataset_hash` | `10479297805450667706` | same | same | present with value, identical across all three (expected — shared teacher cache) |
| `split_hash` | `3527444922185093112` | same | same | present with value, identical across all three (expected — `split_seed=42` fixed by design) |
| `teacher_cache_fingerprint` | — | — | — | **absent entirely** as a named field; inferable only indirectly from `train_b*.log` (`cache_hits=10381`/`cache_misses=0` for all three in `epoch3.meta.json`, confirming a fully shared, fully warm cache — but no fingerprint/hash value is recorded anywhere) |
| `architecture` | `"INPUT=2420 L1=256 L2=32"` | same | same | present with value |
| `init_seed` | `42` | `43` | `44` | present with value (the one deliberately-varied field) |
| `split_seed` | `42` | `42` | `42` | present with value, identical by design |
| `shuffle_seed` | `null` | `null` | `null` | present but null (field exists, value intentionally unset per pre-registration — not a gap) |
| `epochs` | `3` | `3` | `3` | present with value |
| `selected_epoch` | — | — | — | **absent entirely** as a literal field; inferable from `train_b*.log`'s `best (valid_loss=…) → …best.bin (from …epoch3.bin)` line — best == epoch3 == final for all three |
| `lr_schedule` | `"StepHalf"` | same | same | present with value |
| `wdl_lambda` | `null` | `null` | `null` | **present but null** for all three epochs, all three arms |
| `valid_cp_mse` | `null` (literal JSON null) | `null` | `null` | **present but null** for all three — the aggregate `valid: loss_raw=…` line in `train_b*.log` (B1=209449.95, B2=210353.60, B3=211685.39 at epoch3) is a related-but-differently-named quantity, not this field |
| `valid_wdl_loss` | `null` | `null` | `null` | present but null, all three |
| `ft_dead_neurons` | `0` | `0` | `0` | present with value, identical |
| `l2_dead_neurons` | `12` | `12` | `16` | present with value — **B3 differs from B1/B2** (12 vs. 12 vs. 16); the one metric where seed variation visibly shows up in this table |
| `weight_variance` | — | — | — | **absent entirely** — this is a byte-parse of the weight file (as `docs/weights_registry.toml`'s existing entries compute it), not a training-log field; out of scope to compute today |

## B1-vs-A gate: zero games played, despite the directory name

`results/phase_a2/b1_vs_a/` implies a result exists. **It does not.** Per `SUSPENDED.md` and `state.json`, read directly:
- **Games launched: 0.** All 1700/1700 shards are `"status": "pending"`. `confirmed_prefix: 0`. `decisive_verdict: null`.
- No shard ever advanced past its initial paused state — the resource monitor held every launch back from the first loop iteration, first on load average (18.3), then on swap (~85.7%, steady, never dropping below the default 50% pause threshold).
- No `combined.json`/`combined.jsonl` exists (only written once at least one shard's result is confirmed).
- Suspension was an explicit **user decision**, not a crash, not an auto-stop from hitting a verdict, and no background auto-resume is active.
- Manifest values are fully recorded for whenever this resumes: B1 sha256 `019d13f284447b6afc3905dfccb7a5a570e4e3d3b08655a7f3a7b43b174a1385`, A (`data/weights_v011_opening_combined.bin`) sha256 `a45be6099c0936283e79f34d380a4dbc7ba681796bb0bb56b2cd743c2c786ea6`, corpus `data/gate/openings_gateB.sfen` (1707 positions, sha256 `816fdf7661989b348bf1c2e078fd6b5748ff9cfc14fa0aed3b83c6df39d56545`), SPRT bounds elo0=0/elo1=20/α=0.05/β=0.05, trinomial paired-by-id.

**Say this in plain words for anyone reading only the directory name: zero evidence currently exists about whether B1 beats A. Nothing today's audit found changes that.**

## Preflight items (listed only — none run today)

All of the following are individually cheap (a `shasum` on a 1.24 MB file, a byte-parse for variance) but are still out of scope for a "manifests only, no compute" audit:

1. **sha256 for B2 and B3's final weight files** — only B1 has one recorded (in `SUSPENDED.md`, not even in a per-arm meta file).
2. **`weight_variance` for all three arms** — needs a byte-parse of each `.bin`, same method `docs/weights_registry.toml`'s existing entries used.
3. **A `teacher_cache_fingerprint` field** — currently inferable only indirectly (`cache_hits`/`cache_misses` in the meta JSON); worth promoting to an explicit recorded field in the training pipeline's own output so future audits don't need to cross-reference log lines.
4. **Resolve the swap-pause threshold before resuming `b1_vs_a`** — steady ~85.7% swap never cleared under the default `--max-swap-pct 50`; `SUSPENDED.md` notes a ~92% value was discussed but never applied. This is a decision for whoever resumes the gate, not something this audit resolves.
5. **Add B1/B2/B3 entries to `docs/weights_registry.toml`** — only after item 1 (sha256) exists; the existing v011 entry is the schema template to follow.

## What this audit does not do

Does not propose B2 or B3 as an alternative candidate to B1. Does not run, resume, or estimate the outcome of the `b1_vs_a` gate. Does not compute any preflight item listed above. Does not modify `phase_a2_seeded_init_preregistration.md`, any weight file, or `docs/weights_registry.toml`.
