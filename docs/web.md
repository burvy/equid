# Web app (web/): how it is built
One cost field, three views. `web/src/main.ts` is glue only; all math is in the engine.

- **Data flow:** controls → metric JSON → `engine.field(origin, metric, t)` (Dijkstra) → per-node and per-place costs →
  `engine.view_buffers(field, origin, warp, lift, metresPerS, budgetS, hideBeyond)` → typed arrays → deck.gl layers
  (`COORDINATE_SYSTEM.LNGLAT_OFFSETS`) over a MapLibre basemap (OpenFreeMap positron, no key).
- **Views:** `map` (warp 0, flat), `warp` (warp 1: bearing kept, radius = your cost; rings every N min), `3d` (height = cost).
  `setView` animates warp/lift between them.
- **Which controls call the engine:** weights, mode, time and origin re-run `field`; budget, ratings α and view only
  re-run `view_buffers` / the list.
- **main.ts sections:** datasets (auto-discovers every `data/<region>/` with graph.json + places.json; fixture = fallback) ·
  state `S` · map + deck · engine (`runField`) · rendering (`layers`, list, formula, legend) · views · controls · loading.
- **URL params** for demos: `?data=princeton&view=warp&preset=night&t=1320&origin=123`. `window.equid` holds the state in devtools.
- Presets come from `data/presets.json`; the sliders come from `Engine.features_json()`. Adding a feature in the engine adds a slider.
- **AI trip panel** (top of the sidebar): text box + 3 example chips → `POST /agent` (Vite proxies `/agent` to
  `cargo run -p server` on :8787; `EQUID_AGENT=http://host:port npm run dev` or `?agent=<url>` to change). The answer's metric
  becomes the `Custom (AI)` preset (sliders, formula, field), its stops are numbered amber markers in every view, and the
  pitch + plain-English weights show in the panel. Run the server on the same data as the page: `EQUID_DATA=data/princeton`.
