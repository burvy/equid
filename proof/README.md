# proof: every Equid metric is a valid shortest-path metric (Lean 4)

Equid lets anyone (or the AI) make a route metric by picking weights for 20 street features:
cost of an edge = Σ aᵢ · fᵢ. Dijkstra's algorithm finds true shortest paths only if every edge cost is
**>= 0**. This folder proves that in Lean 4 for all 20 features and any weights aᵢ >= 0. It also
**generates the engine's Rust feature code from the same formulas it proves**.

```sh
cd proof
lake build      # checks every proof (~10 s, no Mathlib needed); prints the axioms used
lake exe gen    # writes ../engine/src/metrics_gen.rs from the proven formulas
cargo test -p engine metrics_gen   # generated == hand-written, bit for bit, on all Princeton edges
```

## The idea in one breath
The features are written once, in a tiny formula language with two kinds of formulas:
`S` (signed, may be any number) and `N` (non-negative). `N` only has constructors that cannot go
below zero: sums, products and quotients of `N`s, `max(s, 0)`, `clamp(s, 0, 1)`, `1 - clamp(s, 0, 1)`,
min/max, if-then-else. One induction (`N.eval_nonneg`, Expr.lean) proves that every `N` formula is
>= 0. So each feature, being an `N`, is non-negative by its type. The same terms are then printed as
Rust (`N.rust`), so the code the engine runs is the formula that was proven, not a retyped copy.

## What is proven (Equid/Features.lean)
| theorem | says |
|---|---|
| `N.eval_nonneg` | every `N` formula is >= 0, for **any** values of the signed readings (grade, shade, flood, ...), counts and flags, as long as length, climb and speed are >= 0 |
| `features_nonneg` | each of the 20 features (time, climb, steep, steps, rough, busy, unlit, sun, exposed, stress, bench/toilet/water gap, icy, flood, crossing, no_sidewalk, kerb, dull, stop) is >= 0 |
| `cost_nonneg` | Σ aᵢ · fᵢ >= 0 for every weight list with aᵢ >= 0: every finder and AI-made metric is a valid Dijkstra metric |
| `junction_charge_nonneg` | the street crossings the loader derives at junctions, ½ · (Σ class weights − max), are >= 0 |
| `object_term_nonneg` | object layers (`near:<label>`) add w · len · near / 500 >= 0 |

The only assumptions: length, climb and descent are >= 0 (the engine's `Graph::validate` rejects
anything else), and speed is a positive constant. The `#guard` spot checks also confirm the
formulas mean what the docs say (a 10 % slope counts its full length as steep, a shaded icy slope at
night stays fully icy, 1 unsignalled + 1 signalled crossing = 1.3).

## What is NOT proven (say it before a judge does)
- **Floating point.** The proofs are over exact rationals; the engine uses `f32`. (IEEE sums,
  products and quotients of non-negative floats are non-negative too, but that is not proven here.)
- **The data.** That a street really is lit, flooded or shaded is data quality, not mathematics.
- **The reader.** `inputs()` in metrics_gen.rs (field reads, Option defaults, the shade slot for the
  time of day) is hand-written Rust. Non-negativity does not depend on it: the theorem holds for any
  value it could return.
- **Dijkstra itself** is the textbook algorithm (petgraph); we prove its precondition, not the algorithm.
- Impassable edges (`forbid`) cost +∞. That is outside the sum and fine for Dijkstra.

## Generated == hand-written
`engine/src/metrics_gen.rs` has a drop-in `edge_features` with the same signature as
`features::edge_features`. Its test checks both give **bit-identical** f32 values for every Princeton
edge, both directions, foot and bike, at 7 times of day. With the engine switched to the generated
function, the whole engine test suite gives identical results (36 pass, 2 ignored).
If a feature formula changes in features.rs, change it in Features.lean too, then run `lake exe gen`:
the bit-equality test catches any drift.
