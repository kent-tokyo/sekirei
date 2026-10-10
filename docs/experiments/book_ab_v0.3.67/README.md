# Opening-book paired diagnostic (v0.3.67)

This compact bundle records a real `UseBook=false` versus `UseBook=true`
paired diagnostic. Both arms used the same two held-out positions, color
reversal, material evaluation, one thread, 50 ms byoyomi, and a 64-ply cap.
The book was built only from Sekirei self-play CSA records.

The four games in each arm were all capped draws. The result is therefore
**inconclusive and is not playing-strength evidence**. Its purpose is to prove
the evidence contract: 128 decision rows and four terminal rows per arm join
cleanly; the on arm records two book selections and its fallbacks; all hashes,
settings, commands, and costs are retained.

Validate and reproduce the summary with:

```sh
python3 scripts/validate_book_ab_bundle.py \
  docs/experiments/book_ab_v0.3.67/manifest.json \
  --report docs/experiments/book_ab_v0.3.67/report.json
```

The engine is the exact `v0.3.67` tag. The match runner includes the later
`gameover` notification fix so decision rows receive terminal records.
