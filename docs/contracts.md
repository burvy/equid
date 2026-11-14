# Equid contracts

The interfaces between **data** (prep → files), **engine** (Rust → WASM) and **web** (Vite + TS).
If you change one, update this file, the Rust types, and the fixture in the same commit.

The Rust types are the source of truth for the file formats: `engine/src/graph.rs` (graph + places),
`engine/src/metric.rs` (metric / preset), `engine/src/features.rs` (feature list). A file is valid iff
`Core::load` accepts it (it parses and runs `validate()`).

```
prep/ (once) ──► data/<region>/graph.json, places.json ──► engine (WASM, in the browser) ──► web views
                 data/presets.json ─────────────────────► engine (embedded) + web (preset picker)
```

Conventions: coordinates are WGS84 `[lon, lat]` (lon first, like GeoJSON). Lengths and elevations are in metres.
Time of day `t` is **local minutes since midnight** (0–1439, wraps). JSON is plain JSON (no comments).

---

## 1. `graph.json`

Path: `data/<region>/graph.json` (real: `data/princeton/`, toy: `data/fixtures/`).

```jsonc
{
  "meta": {
    "version": 1,
    "region": "princeton",
    "bbox": [-74.68, 40.33, -74.64, 40.36],   // [west, south, east, north]
    "tz": "America/New_York",
    "date": "2026-11-14",                      // demo date for sun times and shade slots
    "sunrise_min": 404, "sunset_min": 1003,    // local minutes since midnight (06:44, 16:43)
    "slot_min": 30, "slots": 48,               // time-dependent arrays: 48 slots of 30 min from 00:00
    "generated": "2026-11-14T05:00Z",          // optional
    "source": "OSM Overpass + Open-Meteo elevation"  // optional
  },
  "nodes": [ { "lon": -74.662, "lat": 40.348, "elev_m": 40.0, "osm": 105031 }, ... ],
  "edges": [ { ...see below... } ]
}
```

**Nodes** are intersections and dead ends only; the array index is the node id used everywhere
(`edges[].from/to`, `places[].node`, `field` input and output). `osm` = OSM node id (optional, for debugging).

**Edges** are stored **once (undirected)**. Direction-dependent fields are given for `from → to`;
the engine swaps them for `to → from`. One example (all fields):

```json
{
  "from": 0, "to": 1, "osm": 11807451,
  "length_m": 100.4,
  "geom": [[-74.662, 40.348, 40.0], [-74.66141, 40.34802, 40.5], [-74.66082, 40.348, 40.0]],
  "climb_m": 0.5, "descent_m": 0.5, "max_grade_pct": 1.0,
  "highway": "primary",
  "foot": true, "bike": true, "oneway_bike": 0,
  "steps": false,
  "surface": "paved", "surface_raw": "asphalt",
  "lit": true, "covered": false,
  "busy": 3, "cycleway": "none", "lts": 4,
  "wheelchair": null,
  "near_bench_m": 120, "near_toilets_m": 300, "near_water_m": 500,
  "tree": null,
  "shade": null
}
```

| field | type | meaning |
|---|---|---|
| `from`, `to` | u32 | node indices |
| `osm` | i64? | OSM way id (debug) |
| `length_m` | f32 ≥ 0 | length along `geom` |
| `geom` | `[[lon,lat,elev_m],…]` | shape polyline `from → to`, **includes both endpoints**, ≥ 2 points |
| `climb_m`, `descent_m` | f32 ≥ 0 | total up / down along `geom`, in the `from → to` direction |
| `max_grade_pct` | f32 ≥ 0 | steepest absolute grade in %, smoothed over ≥ 10 m |
| `highway` | string | OSM `highway` value (footway, path, residential, primary, steps, cycleway, …) |
| `foot`, `bike` | bool | may walk / may cycle here |
| `oneway_bike` | 0 / 1 / -1 | 0 both ways, 1 bike only `from → to`, -1 only `to → from` (walking ignores it) |
| `steps` | bool | stairs |
| `surface` | `"paved"`/`"compacted"`/`"unpaved"`/`"rough"`/null | normalised; `rough` = cobbles, sett, unhewn stone, roots. null = unknown |
| `surface_raw` | string? | the OSM `surface` tag as-is |
| `lit` | bool? | lit at night; null = unknown. OSM `lit` tag when present, else inferred (see `lit_src`) |
| `lit_src` | string? | `"tag"` OSM lit tag, `"lamp"` street lamp within 25 m, `"road"` tertiary-or-bigger road or a path within 15 m of one, `"campus"` footway/steps on Princeton University grounds, `"trail"` unpaved/untagged path, track or bridleway away from roads (lit = false). Rules in prep/src/night.rs |
| `covered` | bool | under a roof: `covered=yes`, tunnel, arcade |
| `busy` | 0–3 | adjacent traffic: 0 path away from roads, 1 residential/service, 2 tertiary/secondary, 3 primary/trunk |
| `cycleway` | `"track"`/`"lane"`/`"shared"`/`"none"`/null | best cycling infrastructure on the edge |
| `lts` | 1–4 / null | bike Level of Traffic Stress (1 = kids OK, 4 = fast traffic); null if `bike` is false or unknown |
| `wheelchair` | string? | OSM `wheelchair` tag as-is |
| `near_bench_m`, `near_toilets_m`, `near_water_m` | f32, 0–500 | distance from the edge to the nearest bench / toilets / drinking water, **capped at 500** (500 = none nearby). Missing = 500 |
| `flood` | f32 0–1 / null | share of the edge inside a FEMA 1 % / 0.2 % flood zone (task 027); null = unknown |
| `crossings`, `crossings_unsignalled` | u8 | marked crossings on the edge, and the subset with no signal or stop (`unsignalled <= crossings`); default 0 |
| `stops` | u8 | stop signs, give-way signs and traffic signals on a road edge (each node charged once); default 0 |
| `sidewalk` | bool / null | on roads: sidewalk present / tagged none / unknown; null on paths |
| `kerb_raised` | u8 | crossings with a raised kerb (wheelchair barrier); default 0 |
| `green`, `water_view` | f32 0–1 / null | share within ~30 m of green space / open water; null = unknown |
| `tree` | f32 0–1 / null | share of the cells within ±2 m of the edge that are tree crown (USGS 3DEP LiDAR, filled by `prep-shade`) |
| `shade` | `[f32; slots]` 0–1 / null | shade per time slot (`meta.slots` values, slot i covers `[i·slot_min, (i+1)·slot_min)`): 1 − share of direct sun reaching head height along the edge on `meta.date` (roofs block, tree crowns dim by season). Night = 1, covered = 1. Filled by `prep-shade` (see prep-shade/README.md) |

Nullable fields may also be omitted. Defaults when omitted: `oneway_bike` 0, `steps` false, `covered` false,
`busy` 0, `near_*` 500, everything else null.

The data stays **metric-agnostic**: it describes the street; the engine turns that into costs (§3).

## 2. `places.json`

Path: `data/<region>/places.json`.

```json
{
  "version": 1,
  "places": [
    {
      "id": "n1", "name": "Toy Café", "lon": -74.65841, "lat": 40.34983,
      "category": "cafe",
      "tags": { "opening_hours": "Mo-Su 07:00-18:00", "wheelchair": "yes" },
      "node": 11, "snap_m": 5.4,
      "rating": 4.6, "n": 812, "rating_src": "yelp"
    }
  ]
}
```

| field | type | meaning |
|---|---|---|
| `id` | string | OSM type letter + id: `n123` (node), `w456` (way, use its centroid) |
| `name` | string? | |
| `lon`, `lat` | f64 | place location |
| `category` | string | one of `bench`, `toilets`, `drinking_water`, `cafe`, `restaurant`, `park`, `viewpoint`, `library`, `pharmacy`, `shelter`, `sunset`, `stargazing` (others allowed; the UI shows unknown ones as "other") |
| `tags` | object | selected OSM tags as-is (`opening_hours`, `wheelchair`, `website`, …) |
| `node` | u32 | nearest graph node (that is where `field` reads the place's cost) |
| `snap_m` | f32 | distance place → node |
| `rating`, `n`, `rating_src` | f32? / u32? / string? | rating (0–5), number of reviews, where it came from. null until task 005 joins ratings |

**Sunset places** (`prep-shade`, appended after prep's places, ids `sunset-1`…`sunset-8`): category `sunset`, `node` = the spot itself
(`snap_m` 0), tags `sun_visible_until` ("16:35", local), `sun_visible_until_min` (minutes), `sunset` ("16:43"), `skyline_deg`
(how high roofs/trees rise toward the setting sun), `date`, `source`. Parks, gardens, nature reserves and viewpoints also get a
`sun_visible_until` tag when the sun is visible from their node in its last 90 minutes.

**Stargazing places** (`prep-shade`, ids `stargazing-1`…`stargazing-8`, at least 400 m apart): category `stargazing`, `node` = the
spot itself (`snap_m` 0), tags `sky_open` (0–1: share of the sky above 10° not hidden by roofs, crowns or hills, from 36 LiDAR
skyline rays), `dark_m` (metres to the nearest lit street; a car street of unknown lighting counts at twice its distance; capped
by the distance to the region edge), `score` (`sky_open` × min(`dark_m` / 250, 1); spots need `sky_open` ≥ 0.7), `source`.

Place cost = cost of its snapped node (the `snap_m` walk is ignored for now; it is the same for every metric).

## 2b. `objects.json` (open-vocabulary objects, task 015)

Path: `data/<region>/objects.json` (optional). Any plain-language label can become a street feature: the file is just
the points where that kind of object was seen (OSM tags, Vexcel `elements/detect`, ...).

```json
{ "version": 1, "objects": [ { "label": "playground", "lon": -74.6591, "lat": 40.3487, "src": "vexcel", "conf": 0.83 } ] }
```

`label` is trimmed + lower-cased by the engine (free text: "dog park", "trash can"). `src` and `conf` are optional.
The engine derives, per edge, the distance from the edge geometry to the nearest object of each label (metres, **capped at 500**,
like `near_bench_m`), so no per-edge data is stored in `graph.json`. A live detect just adds points
(`Core::add_objects` / `Engine.add_objects`). Offline cache of a label menu: `cargo run -p server --bin build_objects`.

**Metric weight `near:<label>`** (any label with object data): cost term = `weight x length x distance_to_nearest / 500`
(>= 0, so Dijkstra / the Lean proof still hold). Positive weight = "I want to be near these" (cost grows with distance
from them). `near:bench`, `near:toilets`, `near:water` are the built-in `bench_gap` / `toilet_gap` / `water_gap`.
Weighting a label with no data is an error from `field` (never silently free). Not forbiddable.

## 3. Features, metrics and presets

Edge cost under a metric:

```
cost(edge, direction, t) = Σᵢ aᵢ · fᵢ(edge, direction, t)      aᵢ ≥ 0, fᵢ ≥ 0  ⇒  cost ≥ 0
```

`cost ≥ 0` is exactly what Dijkstra needs to be correct (and what the Lean proof states). An edge is
**impassable** (skipped) if the mode may not use it (`foot`/`bike`/`oneway_bike`) or if a **forbidden**
feature is > 0 on it (wheelchair forbids `steps`).

Units: `time` is in seconds; every other feature is in metres (metres of travel that "count", or metres climbed).
A weight `aᵢ` therefore reads as *seconds of penalty per metre*, and total cost is **seconds-equivalent**
(the UI shows it as minutes).

| id | unit | fᵢ (for one edge, one direction, time t) |
|---|---|---|
| `time` | s | `length_m / speed`; speed = 1.34 m/s (foot, 4.8 km/h) or 4.44 m/s (bike, 16 km/h) |
| `climb` | m | `climb_m` going forward, `descent_m` going backward |
| `steep` | m | `length · max(0, max_grade_pct − 5) / 5` (0 up to 5 %, full length at 10 %; ADA ramp max is 8.3 %) |
| `steps` | m | `length` if `steps` else 0 |
| `rough` | m | `length · r`, r = paved 0, compacted .33, unpaved .67, rough 1, unknown .2 |
| `busy` | m | `length · busy / 3` |
| `unlit` | m | at night only (`t < sunrise_min` or `t ≥ sunset_min`): `length · u`, u = unlit 1, unknown .5, lit 0 |
| `sun` | m | by day only: `length · (1 − shade)`, shade = `shade[slot(t)]` if present, else 1 if `covered`, else `tree` or 0 |
| `exposed` | m | `length` if not `covered` (rain) |
| `stress` | m | `length · (lts − 1) / 3`, unknown lts counts .5 |
| `bench_gap` | m | `length · near_bench_m / 500` |
| `toilet_gap` | m | `length · near_toilets_m / 500` |
| `water_gap` | m | `length · near_water_m / 500` |
| `icy` | m | `length · clamp((max_grade_pct − 3) / 4, 0, 1) · s`; s = shade(t) by day (as `sun`), 1 at night. Shady hills stay icy |
| `flood` | m | `length · flood` (null = 0) |
| `crossing` | n | `crossings_unsignalled + 0.3 · (crossings − crossings_unsignalled)`; weight = seconds per crossing |
| `no_sidewalk` | m | `length` if `sidewalk == false` (unknown = 0) |
| `kerb` | n | `kerb_raised` (use with `forbid` for a no-raised-kerb wheelchair route) |
| `dull` | m | `length · (1 − max(green, water_view))`; 0 when both are null |
| `stop` | n | `stops`; weight = seconds per stop (bike preset: 8) |

Every new feature is 0 when its data is missing, so a region without the 027 fields behaves exactly as before.

The feature list is **append-only** (the index is the position in the feature vector).
Adding a feature = new row here + `FEATURES` entry + one line in `edge_features`.

### Metric JSON (what `field` takes)

```json
{ "mode": "foot", "weights": { "time": 1, "climb": 15, "steep": 3, "rough": 2 }, "forbid": ["steps"] }
```

- `mode`: `"foot"` (default) or `"bike"`: picks base speed and which edges are allowed.
- `weights`: feature id → aᵢ. Missing = 0. Unknown id, negative, NaN or infinite → error.
- `forbid`: feature ids; an edge with that feature > 0 is impassable.

### Presets: `data/presets.json`

An array of named metrics (a metric plus `id`, `name`, `description`). A preset object can be passed to
`field` directly. The engine embeds the file (`Engine.presets_json()`), so the engine and web never disagree.
Current presets: walk, hiking (Naismith), wheelchair, stroller, shade, night, rest, rain, bike, icy, peaceful, flood, crossings. Wheelchair and stroller also weigh `kerb` and `no_sidewalk`.
Weights are first guesses, to be tuned on the real data.

```json
{ "id": "wheelchair", "name": "Wheelchair", "description": "No stairs; …",
  "mode": "foot", "weights": { "time": 1, "climb": 15, "steep": 3, "rough": 2 }, "forbid": ["steps"] }
```

## 4. WASM API (engine → web)

Built with wasm-bindgen (`--target web`) into `web/src/pkg/` (`npm run wasm`). TypeScript types are generated.

```ts
import init, { Engine } from './pkg/engine.js';
await init();

const engine = new Engine(graphJsonText, placesJsonText); // throws on invalid files
engine.node_count(); engine.place_count();
const origin = engine.nearest_node(lon, lat);             // snap a click to a node id

const f = engine.field(origin, JSON.stringify(metricOrPreset), tMin); // throws on a bad metric
f.node_cost;   // Float32Array, one per node (graph order), seconds-equivalent, Infinity = unreachable
f.place_cost;  // Float32Array, one per place (places.json order)
f.free();      // release the wasm memory when done

Engine.features_json(); // '[{"id":"time","unit":"s","desc":"…"}, …]' in feature order (for sliders)
Engine.presets_json();  // contents of data/presets.json
```

Each getter returns a fresh copy; read it once into a variable.
The web does all the cheap math (λ, ratings exponent α, heights, warp radius) on these arrays, so
moving those sliders does not call the engine. Changing weights, mode or time does.

**How `field` works:** at load the engine builds a directed petgraph (each undirected edge becomes two arcs).
Per call it computes every arc's cost with `Metric::edge_cost` at time `tMin` (the departure time; the whole trip is
costed at that time), drops impassable arcs, and runs `petgraph::algo::dijkstra` from the origin. Unreachable
nodes/places are `Infinity`; a place's cost is its snapped node's cost. Timing on a synthetic 19.8k-edge grid
(hiking preset): ~3.3 ms native release, ~3.7 ms in wasm (node), so it is fine to call on every slider move.

### Open-vocabulary objects in the WASM API

```ts
engine.load_objects(objectsJsonText);          // -> number of objects; enables near:<label> weights for those labels
engine.add_objects('playground', JSON.stringify([[lon, lat], ...])); // -> total objects for the label (live detects)
engine.labels_json();                           // '[{"label":"playground","objects":4,"edges_near":37}, ...]' (the label menu)
```
Features from `features_json()` stay the fixed built-in list; labels come from `labels_json()`.

### View buffers (task 016)

The per-vertex math for the three views lives in the engine (`engine/src/view.rs`); the page hands the arrays to deck.gl.

```ts
const vb = engine.view_buffers(f /* the Field from field() */, origin, warp, lift, metresPerS, budgetS, hideBeyond);
// hideBeyond 0..1 (task 022): beyond-budget streets/places fade (alpha x (1 - h)) and their height/radius is
// pulled to the budget; at 1, edges wholly beyond the budget are omitted. The web uses 0 on the map, 1 in warp/3D.
vb.positions;      // Float32Array xyz per vertex: (dLon deg, dLat deg, z m) OFFSETS from (vb.origin_lon, vb.origin_lat)
vb.colors;         // Uint8Array rgba per vertex (green -> yellow -> red at budgetS, grey beyond / unreachable)
vb.vertex_cost;    // Float32Array seconds-equivalent per vertex (linear along the edge)
vb.start_indices;  // Uint32Array path starts, total vertex count appended (deck binary PathLayer startIndices)
vb.path_edge;      // Uint32Array graph edge index per path (picking); edges with both ends unreachable are omitted
vb.place_positions, vb.place_colors;   // per place, places.json order; unreachable: real position, z 0, grey
vb.origin_lon, vb.origin_lat;          // bbox centre
vb.free();
engine.ring(origin, radiusM);          // Float32Array xy offsets, 97 points, closed (warp-view rings)
engine.isochrone(f, origin, costS, warp, lift, metresPerS); // Float32Array xyz offsets, closed: where the field crosses costS
// (outermost crossing per 2-degree bearing bin, gaps filled in polar coords). Same blend as view_buffers, so it is an
// isochrone on the map and the circle of radius costS x metresPerS at warp 1 (task 024). Empty if nothing reaches costS.
```

- `warp` 0..1 blends real -> warped: `warped = origin + direction-to-vertex x (cost x metresPerS)` (bearing kept, radius = cost).
- `lift` = metres of height per second of cost (z); 0 = flat.
- Use `coordinateSystem: COORDINATE_SYSTEM.LNGLAT_OFFSETS` and `coordinateOrigin: [vb.origin_lon, vb.origin_lat]`
  on the layers: absolute f32 lon/lat would jitter by ~1 m. Pass `startIndices` + `attributes.getPath {value: positions, size: 3}`,
  `getColor {value: colors, size: 4}`; set `_pathType: 'open'`.
- Timing (19.8k edges, 79k vertices): ~2.1 ms native release; ~9 ms in wasm incl. the 5 typed-array copies.

Native Rust (tests, a future server) uses the same logic through `engine::Core`
(`Core::load`, `nearest_node`, `field(origin, &Metric, t)`).

## 5. Fixture

`data/fixtures/graph.json` + `places.json`: a hand-made 4 × 3 grid (12 nodes, 17 edges, 6 places, ~100 m spacing)
near Palmer Square. It is **not real data**. It covers every edge field: a busy oneway-for-bikes primary road, stairs,
an unlit footway, a gravel path, a covered walkway with a full `shade` array, a sett (rough) pedestrian street,
and an edge with unknown surface/lit. Use it in unit tests and for web work before the real data lands.

## 6. Trip agent server (`POST /agent`)

`cargo run -p server` (axum, `server/`): the only server we run. Plain English -> weight vector + stops + pitch, by
Claude tool use (no framework). Env: `ANTHROPIC_API_KEY` (read from the environment only; never logged or committed;
without it a deterministic **mock** model drives the same loop, and the response says `"llm":"mock"`),
`EQUID_MODEL` (default `claude-sonnet-5-5`), `EQUID_DATA` (dir with graph.json + places.json, default `data/fixtures`),
`PORT` (default 8787), `VEXCEL_API_KEY` or a file `VEXCEL_ENV_FILE` (default `Z:\App Data\Downloadsexcel.env`, the bare key or `VEXCEL_API_KEY=...`; never printed) for live detects; `objects.json` next to the graph is loaded at start. CORS is open (dev).

```jsonc
// request
{ "text": "stroller, want a coffee, it's hot", "origin": {"lon": -74.662, "lat": 40.348},  // or {"node": 12}; default: bbox centre
  "t": 840 }                                                                                // local minutes since midnight, default 720
// response
{ "metric": {"mode":"foot","weights":{"time":1,"climb":10,"steep":3,"rough":3},"forbid":["steps"]},  // a valid §3 metric: pass to engine.field
  "stops": [ {"id":"n1","name":"Toy Caf�","category":"cafe","minutes":13.5,"why":"..."} ],
  "pitch": "2-4 sentences, realtor tone",
  "llm": "anthropic:claude-sonnet-5-5", "tools_called": ["list_features","make_metric", "..."],
  "detected": [ {"label":"playground","lon":-74.659,"lat":40.349,"src":"vexcel","conf":0.8} ] }   // objects found live: web calls engine.add_objects
// errors: 502 {"error": "..."} (model/API failure, no finish, bad origin)
```

Tools the model gets: `list_features`, `list_places(categories)`, `make_metric(mode, weights, forbid)` (validated by
`Metric::compile`: weights >= 0), `field(categories, limit)` (cheapest places, minutes, computed natively by `engine::Core`),
`list_labels` (the object label menu: `Core::labels`), `detect(label, radius_m)` (live Vexcel detect in a small square
around the origin, adds the label; `near:<label>` can then be weighted), `explain(place_id)` (route length + per-feature share of the cost on the cheapest route, `Core::explain`), and `finish(stops, pitch)`.
The model's only outputs are a weight vector and place ids, so every cost comes from the engine over non-negative weights.
**Grounding check:** `finish` is rejected (and the model must retry) if any number in the pitch is absent from the request
or the tool results; stop ids must exist. Costs are quoted in minutes of personal cost.
