from component_benchmark_preflight import preflight
from run_component_benchmark import thermal_status


def test_thermal_status_reads_only_explicit_warnings():
    def no_warning(*_args, **_kwargs):
        return "Note: No thermal warning level has been recorded\nNote: No performance warning level has been recorded\n"

    assert thermal_status(no_warning, "Darwin") == {
        "thermal_warning": "none",
        "performance_warning": "none",
    }


def test_preflight_requires_every_declared_condition():
    passing = preflight(2.0, True, 1.5, "ac", {
        "thermal_warning": "none", "performance_warning": "none",
    })
    assert passing["verdict"] == "PASS"
    refused = preflight(2.0, True, 2.1, "battery", {
        "thermal_warning": "unknown", "performance_warning": "reported",
    })
    assert refused["verdict"] == "REFUSE"
    assert all(not check["pass"] for check in refused["checks"].values())
