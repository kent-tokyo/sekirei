# Historical run-specific scripts

Scripts here retain the exact commands and inputs for completed experiments.
They are not general-purpose entry points. Run them only after checking that
their pinned inputs, binaries, and output directories are available and match
the original experiment.

- `extend_gate_sprt.sh`: continuation of the July 2026 conflict-mask SPRT run.
  Its fixed `data/runs/20260717_longrun_conflict_mask` inputs remain protected
  from `cleanup_runs.sh` by the recursive script-reference scan.
