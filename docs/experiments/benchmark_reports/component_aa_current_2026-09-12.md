# Same-binary component A/A noise floor

This report records ten replay captures of the same frozen release binary. It
is a measurement-quality gate, not a comparison against a candidate or a
playing-strength result.

- Source head: `34cf6beea5d08ab1f10c3909d5320ddc49753bb0`
- Binary SHA-256: `ada7fb164b106ff482e8a0a8ee0eec298aa05ae778a2f364a1c0e918cea5f3f3`
- Captures: 10, paired as 5 adjacent A/A pairs
- Cases: 66; samples per case: 21; target/minimum window: 50/20 ms
- Worktree status recorded by every capture: clean
- Raw captures: `/private/tmp/sekirei-component-aa-current/aa01` through `aa10`
- Aggregate JSON: `/private/tmp/sekirei-component-aa-current/aggregate.json`

## Aggregate

| Measure | Result |
|---|---:|
| Overall geomean | 0.9989x |
| Pair geomean 95% CI | 0.9943–1.0036x |
| Pair geomeans | 0.9956x, 1.0020x, 0.9968x, 1.0039x, 0.9963x |
| Case median range | 0.9760–1.0714x |

The overall pair interval is inside the preregistered 0.98–1.02 noise-floor
window. The case-level spread is retained; no outlier was removed. Candidate
adoption still requires the independent baseline/candidate sessions and all
correctness gates.
