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

## Web app (web/)
One cost field, three views. `web/src/main.ts` is glue only; all math is in the engine.

- **Data flow:** controls → metric JSON → `engine.field(origin, metric, t)` (Dijkstra) → per-node and per-place costs →
  `engine.view_buffers(field, origin, warp, lift, metresPerS, budgetS)` → typed arrays → deck.gl layers
  (`COORDINATE_SYSTEM.LNGLAT_OFFSETS`) over a MapLibre basemap (OpenFreeMap positron, no key).
- **Views:** `map` (warp 0, flat), `warp` (warp 1: bearing kept, radius = your cost; rings every N min), `3d` (height = cost).
  `setView` animates warp/lift between them.
- **Which controls call the engine:** weights, mode, time and origin re-run `field`; budget, ratings α and view only
  re-run `view_buffers` / the list.
- **main.ts sections:** datasets (auto-discovers every `data/<region>/` with graph.json + places.json; fixture = fallback) ·
  state `S` · map + deck · engine (`runField`) · rendering (`layers`, list, formula, legend) · views · controls · loading.
- **URL params** for demos: `?data=princeton&view=warp&preset=night&t=1320&origin=123`. `window.equid` holds the state in devtools.
- Presets come from `data/presets.json`; the sliders come from `Engine.features_json()`. Adding a feature in the engine adds a slider.

## Status
Work in progress (hackathon build). The engine's `field` is real (petgraph Dijkstra over user-weighted, time-dependent edge costs).

## Credits
Map data © OpenStreetMap contributors (ODbL).

## AI trip agent

```
export ANTHROPIC_API_KEY=sk-ant-...      # optional: without it a mock model runs the same tool loop
cargo run -p server                      # http://127.0.0.1:8787, data from data/fixtures (EQUID_DATA to change)
curl -X POST localhost:8787/agent -H 'content-type: application/json'   -d '{"text":"stroller, want a coffee","origin":{"lon":-74.662,"lat":40.348},"t":600}'
cargo test -p server                     # the three scripted prompts through the whole loop (mock)
```
Details: docs/contracts.md section 6.
