#!/usr/bin/env python3
"""Register a match process group before exec, then await its one-shot release.

This small process occupies the match runner's PID and process group. The
supervisor can stop that exact group if the Python gate wrapper disappears.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import time

from sekirei_tools.artifacts import atomic_write_json


def main() -> int:
    channel = Path(os.environ["SEKIREI_AB_SUPERVISOR_CHANNEL"])
    token = os.environ["SEKIREI_AB_SUPERVISOR_TOKEN"]
    wrapper_pid = int(os.environ["SEKIREI_AB_WRAPPER_PID"])
    command = sys.argv[1:]
    if (not command or os.name != "posix" or os.getpgid(0) != os.getpid()
            or os.getppid() != wrapper_pid):
        return 2
    atomic_write_json(channel / "runner.json", {
        "token": token,
        "pid": os.getpid(),
        "pgid": os.getpgrp(),
        "wrapper_pid": wrapper_pid,
        "command": command,
    })
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if os.getppid() != wrapper_pid:
            return 3
        try:
            release = json.loads((channel / "release.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            time.sleep(0.02)
            continue
        if release.get("token") == token and release.get("pid") == os.getpid():
            if os.getppid() != wrapper_pid:
                return 3
            child_env = dict(os.environ)
            for key in ("SEKIREI_AB_SUPERVISOR_CHANNEL", "SEKIREI_AB_SUPERVISOR_TOKEN",
                        "SEKIREI_AB_WRAPPER_PID"):
                child_env.pop(key, None)
            os.execvpe(command[0], command, child_env)
        return 4
    return 5


if __name__ == "__main__":
    sys.exit(main())
