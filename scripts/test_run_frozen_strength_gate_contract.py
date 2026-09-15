from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WRAPPER = (ROOT / "scripts/run_frozen_strength_gate.sh").read_text(encoding="utf-8")
SPRINT = (ROOT / "scripts/sprint_gate.sh").read_text(encoding="utf-8")


def test_frozen_wrapper_forwards_all_protocol_inputs_from_plan():
    assert '"openings"]["path"]' in WRAPPER
    assert '"protocol"]["byoyomi_ms"]' in WRAPPER
    assert '"engine_options"]["SearchMode"]' in WRAPPER
    # The frozen wrapper uses the durable orchestrator directly; it must not
    # fall back to the legacy SPRINT environment-variable bridge.
    assert '--corpus "$openings"' in WRAPPER
    assert '--byoyomi "$byoyomi_ms"' in WRAPPER
    assert '"SearchMode=$search_mode"' in WRAPPER


def test_sprint_runner_uses_forwarded_openings_time_and_search_mode():
    assert 'BYOYOMI_MS=${BYOYOMI_MS:-1000}' in SPRINT
    assert 'SEARCH_MODE=${SEARCH_MODE:-Speculative}' in SPRINT
    assert '--positions "$SHARD" --games-per-position "$GAMES_PER_POSITION" --byoyomi "$BYOYOMI_MS"' in SPRINT
    assert '"SearchMode=$SEARCH_MODE"' in SPRINT


def test_frozen_wrapper_runs_preflight_before_it_can_exec_the_gate():
    """A resource refusal must abort before a build or a match can start."""
    preflight = WRAPPER.index("python3 scripts/gate_resource_preflight.py")
    launch = WRAPPER.index("python3 scripts/gate_orchestrator.py run")
    assert preflight < launch


def test_frozen_wrapper_records_before_launch_and_can_prepare_without_games():
    initialize = WRAPPER.index("--initialize-only")
    record = WRAPPER.index("record_strength_gate_execution.py")
    launch = WRAPPER.rindex("python3 scripts/gate_orchestrator.py run")
    finalize = WRAPPER.index("finalize_strength_gate_execution.py")
    assert initialize < record < launch < finalize
    assert 'PREPARE_ONLY:-0' in WRAPPER


def test_frozen_wrapper_persists_orchestrator_output_outside_terminal_pipe():
    assert '>> "$outdir/orchestrator.log" 2>&1' in WRAPPER


def test_frozen_wrapper_refuses_to_mix_plans_when_resuming_a_run_directory():
    assert 'existing run directory has a different frozen plan' in WRAPPER
    assert 'cmp -s "$MANIFEST" "$outdir/plan.json"' in WRAPPER
    assert '[ ! -e "$outdir/execution.json" ]' in WRAPPER
