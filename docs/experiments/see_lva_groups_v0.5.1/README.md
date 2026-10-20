# SEE least-attacker groups (default OFF)

The optional `see_lva_groups` core/USI feature replaces only the scalar
least-attacker scan in exact static exchange evaluation. Empty/single
attacker sets use the mailbox; multiple attackers use eleven value-ordered
bitboard groups. Same-value ties retain square order. Compile-time checks
require every piece kind exactly once, equal values within each group and
strictly increasing group values. Gold-shaped pieces are split by value.

The scalar OFF path, exchange gains, occupancy updates, king-recapture stop
rule and reverse propagation retain their original behavior. The feature is
absent from both default feature lists. This is a CPU candidate, with no
pruning, ordering, evaluator, weight or strength change.

Focused tests compare all kind subsets, both colors and square permutations,
promotions/drops, explicit legal/unsafe king recaptures and legal playouts
against the original scalar oracle. The ignored stress compares one selected
move per legal-position visit; visits are not unique positions or a perft proof.

```sh
cargo test -p sekirei-core --features see_lva_groups --lib see_lva_groups
SEKIREI_SEE_LVA_POSITIONS=10000000 cargo test --release -p sekirei-core \
  --features see_lva_groups --lib \
  movegen::see_lva_groups_tests::grouped_see_10m_legal_position_differential \
  -- --ignored --exact --test-threads=1 --nocapture
```

Speed is not yet qualified. The preregistered own-only study uses one fresh
material-generated 32-position corpus, native OFF/ON builds without PGO,
tune or counters, exact 1T depth12 ABBA, then a single independent 8T H1.
H1 uses Hash64 MiB,1000ms,first host-clean whole ABBA, five quiet1Hz samples,
maximum eight host retries,1GiB disk floor and no partial statistics. A whole
position is excluded if any arm is below700ms; at least24 positions and a
5000-draw bootstrap lower95% bound above1 are required (seed20261021).
The read-only historical own-r3 HalfKP is a runtime workload, not NNUE
training or model qualification. No external evaluator selects the candidate.
