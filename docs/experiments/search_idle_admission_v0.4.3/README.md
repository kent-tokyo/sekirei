# Opt-in idle admission for future timed diagnostics

`paired_search_diagnostics.py` can wait for consecutive idle process snapshots
before launching a counter-free movetime run. This changes measurement admission;
search code, evaluator and playing defaults are unchanged. See
[usage](../../../scripts/README.md#waiting-for-a-quiet-host-before-timing).

Validation on 2026-10-17, based on main `a1fea9ebe765cacd84f6c1cfee1f5e1bee37e87e`:

- 164 Python regression tests passed, including 22 admission tests with fake
  clocks, fake engine files and mocked search calls.
- Independent review checked strict process parsing, timeout/interruption,
  frozen input revalidation, malformed postflight and default-path compatibility.
- A real preflight-only check used a 15-second deadline, 5-second polling and
  three required idle samples. It admitted after three idle samples. The check
  had no engine dispatch path; completed searches and matches were both zero.
- Local evidence and source copies are in
  `~/.local/state/sekirei/search-experiments/idle-admission-v0.4.3/20261017-preflight-only/`.
  `validation.json` binds the manifest and source hashes.

This is not an NPS, depth or strength result. Earlier closed studies stay closed.
The fixed aspiration independent match gate still requires owner-attested
exclusion identities and a separately qualified run; no new candidate was adopted.
