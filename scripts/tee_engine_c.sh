#!/bin/bash
# Throwaway load-test instrumentation ONLY -- see tee_engine_b.sh for why
# this never runs in the production gate. LOADTEST_LOG_C must be set by the
# caller to a unique path per invocation.
LOG="${LOADTEST_LOG_C:?LOADTEST_LOG_C must be set}"
exec stdbuf -oL ./target/release/sekirei "$@" | stdbuf -oL tee -a "$LOG"
