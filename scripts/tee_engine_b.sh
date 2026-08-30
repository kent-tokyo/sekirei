#!/bin/bash
# Throwaway load-test instrumentation ONLY -- never used in the production
# gate run (adds tee/buffering risk to the timing-sensitive USI stdin/stdout
# pipe that isn't worth taking once real games are being scored). Captures
# every "info depth ..." line from whichever engine is launched through this
# wrapper, so scripts/analyze_loadtest.py can compare NPS/depth/think-time
# serial vs parallel without any code change to sekirei-usi itself.
# LOADTEST_LOG must be set by the caller to a unique path per invocation
# (concurrent parallel-mode invocations must never share one log file).
LOG="${LOADTEST_LOG_B:?LOADTEST_LOG_B must be set}"
exec stdbuf -oL ./target/release/sekirei "$@" | stdbuf -oL tee -a "$LOG"
