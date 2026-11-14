# Equid

A map for how you are right now. Equid measures distance the way *you* experience it
(hills, shade, lit streets at night, benches, a stroller or a wheelchair) and warps the map around you,
so you can see what is really close and which places are worth the trip.

HackPrinceton Fall 2026. Demo region: Princeton, NJ.

## How it works (one paragraph)
Every street segment has features (length, climb, steepness, stairs, surface, traffic, lighting, shade, …).
You pick weights for them (or a preset such as *wheelchair* or *shady*), so a segment costs `Σ aᵢ·fᵢ`
with every weight and feature ≥ 0. The engine (Rust, compiled to WebAssembly, running in your browser)
runs Dijkstra from where you are and gets your personal cost to every street and place.
The interfaces are documented in [docs/contracts.md](docs/contracts.md).

## Layout
| path | what |
|---|---|
| `engine/` | Rust crate: data types, features, metrics, `field`. Builds native (tests) and `wasm32` (browser) |
| `web/` | Vite + TypeScript app that loads the engine as WASM |
| `data/presets.json` | finder presets (weight vectors), embedded in the engine |
| `data/fixtures/` | tiny hand-made graph for tests and early web work |
| `data/<region>/` | real prepared data (`graph.json`, `places.json`) |
| `docs/contracts.md` | file formats, feature list, WASM API |

## Build and run
Needs: Rust with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` **0.2.117** (must match the crate), Node 20+.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.117   # skip if `wasm-bindgen --version` already says 0.2.117

cargo test                                            # engine tests (native)
cargo build --target wasm32-unknown-unknown -p engine # check the wasm build

cd web
npm install
npm run dev      # builds the wasm into web/src/pkg, then serves http://localhost:5173
npm run build    # production build into web/dist (static files)
npm run preview  # serve web/dist
```

## Status
Work in progress (hackathon build). The engine's `field` is real (petgraph Dijkstra over user-weighted, time-dependent edge costs).

## Credits
Map data © OpenStreetMap contributors (ODbL).
