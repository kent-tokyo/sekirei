#!/usr/bin/env python3
"""Between-pair admission and uncensored telemetry for shared-host NNUE gates.

This is not an idle-host NPS/timing qualification. CPU activity never rejects
an in-flight colour-reversed pair or removes its outcomes from a strength
sample. Pin this module and its policy before starting a new trial.
"""
from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import math
import os
from pathlib import Path
import shutil
import subprocess


@dataclass(frozen=True)
class HostPolicy:
    physical_cores: int = 10
    max_foreign_core_fraction: float = 0.3
    minimum_free_mib: int = 512
    stable_samples: int = 3
    poll_seconds: int = 5

    def __post_init__(self) -> None:
        if (type(self.physical_cores) is not int or self.physical_cores <= 0
                or type(self.max_foreign_core_fraction) not in (float, int)
                or not math.isfinite(self.max_foreign_core_fraction)
                or not 0 < self.max_foreign_core_fraction <= 1
                or type(self.minimum_free_mib) is not int or self.minimum_free_mib < 128
                or type(self.stable_samples) is not int or self.stable_samples < 1
                or type(self.poll_seconds) is not int or self.poll_seconds < 1):
            raise ValueError("invalid shared-host policy")

    @property
    def max_foreign_cpu_percent(self) -> float:
        # ps reports 100% per CPU, not 100% for the whole machine.
        return self.physical_cores * 100 * self.max_foreign_core_fraction

    def admission(self, sample: dict) -> bool:
        cpu, free = sample.get("foreign_total_cpu"), sample.get("free_mib")
        return (
            sample.get("cpu_observation_available") is True
            and type(cpu) in (int, float) and math.isfinite(cpu) and cpu >= 0
            and cpu <= self.max_foreign_cpu_percent
            and type(free) is int and free >= self.minimum_free_mib
        )

    def next_stable_count(self, previous: int, sample: dict) -> int:
        return min(previous + 1, self.stable_samples) if self.admission(sample) else 0


def active_pair_action(_sample: dict) -> str:
    """Never censor outcomes using host telemetry, including missing telemetry.

    Engine/protocol errors, corrupt/missing artifacts and actual I/O failures
    still stop the driver. They are not decided by this CPU scheduler.
    """
    return "finish_and_retain_pair"


def parse_processes(payload: str) -> list[dict]:
    lines = payload.splitlines()
    if not lines:
        raise ValueError("empty process observation")
    rows = []
    for line in lines[1:]:
        fields = line.strip().split(None, 3)
        if len(fields) != 4:
            raise ValueError("malformed process observation")
        pid, parent, cpu = int(fields[0]), int(fields[1]), float(fields[2])
        if pid < 1 or parent < 0 or not math.isfinite(cpu) or cpu < 0:
            raise ValueError("invalid process observation")
        rows.append({"pid": pid, "ppid": parent, "cpu": cpu, "command": fields[3]})
    if not rows:
        raise ValueError("empty process list")
    return rows


def foreign_processes(rows: list[dict], own_pid: int) -> list[dict]:
    own = {own_pid}
    while True:
        expanded = own | {row["pid"] for row in rows if row["ppid"] in own}
        if expanded == own:
            break
        own = expanded
    return [row for row in rows if row["pid"] not in own]


def capture(output: Path, policy: HostPolicy | None = None) -> dict:
    policy = policy or HostPolicy()
    sample = {"at": datetime.now(timezone.utc).isoformat(),
              "cpu_observation_available": False, "foreign_total_cpu": None,
              "free_mib": None, "max_foreign_cpu_percent": policy.max_foreign_cpu_percent,
              "cpu_activity_censors_active_pair": False}
    try:
        process = subprocess.run(["ps", "-Ao", "pid,ppid,pcpu,comm"], check=True,
                                 capture_output=True, text=True, timeout=10)
        outside = foreign_processes(parse_processes(process.stdout), os.getpid())
        sample.update(cpu_observation_available=True,
                      foreign_total_cpu=sum(row["cpu"] for row in outside),
                      foreign_busy=[row for row in outside if row["cpu"] >= 70])
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        sample["cpu_observation_error"] = type(error).__name__
    try:
        sample["free_mib"] = shutil.disk_usage(output).free // 1048576
    except OSError as error:
        sample["capacity_observation_error"] = type(error).__name__
    sample["admitted"] = policy.admission(sample)
    return sample
