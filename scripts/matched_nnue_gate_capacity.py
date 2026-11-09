#!/usr/bin/env python3
"""Fail-closed capacity contract for a future matched HalfKP gate.

This module never starts games. A caller must persist every observation and
apply `during_pair` only after retaining the complete colour-reversed pair.
The ten-minute start stability clock begins with the first qualified sample;
it cannot be inferred from a single free-space snapshot.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
import os
from pathlib import Path
import shutil


MIB = 1024 * 1024


@dataclass(frozen=True)
class CapacityPolicy:
    start_local_free_mib: int = 16384
    start_archive_free_mib: int = 8192
    stop_local_floor_mib: int = 8192
    output_cap_bytes: int = 1024**3
    stable_seconds: int = 600
    max_sample_gap_seconds: int = 15

    def __post_init__(self) -> None:
        fields = (self.start_local_free_mib, self.start_archive_free_mib,
                  self.stop_local_floor_mib, self.output_cap_bytes,
                  self.stable_seconds, self.max_sample_gap_seconds)
        if (any(type(value) is not int or value <= 0 for value in fields)
                or self.start_local_free_mib <= self.stop_local_floor_mib):
            raise ValueError("invalid matched gate capacity policy")


def observe(local_path: Path, archive_path: Path, archive_mount: Path) -> dict:
    """One read-only observation; a missing volume or probe fails admission."""
    result = {"local_free_mib": None, "archive_free_mib": None,
              "local_and_archive_same_device": None, "observation_complete": False}
    try:
        # Both paths must exist. An unmounted /Volumes/<name> path cannot be
        # silently interpreted as capacity on the local APFS filesystem.
        local = Path(local_path).resolve(strict=True)
        archive = Path(archive_path).resolve(strict=True)
        mount = Path(archive_mount).resolve(strict=True)
        if (not local.is_dir() or not archive.is_dir()
                or not os.path.ismount(mount)
                or not archive.is_relative_to(mount)):
            raise ValueError("capacity paths must be directories")
        local_device, archive_device = os.stat(local).st_dev, os.stat(archive).st_dev
        result.update(local_free_mib=shutil.disk_usage(local).free // MIB,
                      archive_free_mib=shutil.disk_usage(archive).free // MIB,
                      local_and_archive_same_device=local_device == archive_device,
                      observation_complete=True)
    except (OSError, ValueError, RuntimeError) as error:
        result["observation_error"] = type(error).__name__
    return result


def start_sample_ok(sample: dict, policy: CapacityPolicy) -> bool:
    local, archive = sample.get("local_free_mib"), sample.get("archive_free_mib")
    return (sample.get("observation_complete") is True
            and type(local) is int and local >= policy.start_local_free_mib
            and type(archive) is int and archive >= policy.start_archive_free_mib
            and sample.get("local_and_archive_same_device") is False)


@dataclass(frozen=True)
class StableWindow:
    since: float
    last: float


def advance_window(previous: StableWindow | None, at: float, sample: dict,
                   policy: CapacityPolicy) -> StableWindow | None:
    """A missed or failed observation restarts the ten-minute clock."""
    if type(at) not in (int, float) or not math.isfinite(at) or at < 0:
        raise ValueError("invalid monotonic observation time")
    if not start_sample_ok(sample, policy):
        return None
    if previous is None:
        return StableWindow(float(at), float(at))
    if (not math.isfinite(previous.last) or previous.last > at
            or not math.isfinite(previous.since) or previous.since > previous.last):
        raise ValueError("invalid preceding observation time")
    if at - previous.last > policy.max_sample_gap_seconds:
        return StableWindow(float(at), float(at))
    return StableWindow(previous.since, float(at))


def start_admitted(window: StableWindow | None, at: float, sample: dict,
                   policy: CapacityPolicy) -> bool:
    return (window is not None and type(at) in (int, float) and math.isfinite(at)
            and window.last == at and at >= window.since + policy.stable_seconds
            and start_sample_ok(sample, policy))


def after_pair_action(sample: dict, output_bytes: int,
                      policy: CapacityPolicy) -> str:
    """Decide only whether another pair may start; never discard a result."""
    local = sample.get("local_free_mib")
    if (sample.get("observation_complete") is not True
            or type(local) is not int or local < policy.stop_local_floor_mib
            or type(output_bytes) is not int or output_bytes < 0
            or output_bytes > policy.output_cap_bytes):
        return "stop_after_retaining_pair"
    return "continue_if_other_gates_pass"
