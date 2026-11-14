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
    "sunrise_min": 395, "sunset_min": 1004,    // local minutes since midnight (06:35, 16:44)
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
| `lit` | bool? | OSM `lit`; null = unknown |
| `covered` | bool | under a roof: `covered=yes`, tunnel, arcade |
| `busy` | 0–3 | adjacent traffic: 0 path away from roads, 1 residential/service, 2 tertiary/secondary, 3 primary/trunk |
| `cycleway` | `"track"`/`"lane"`/`"shared"`/`"none"`/null | best cycling infrastructure on the edge |
| `lts` | 1–4 / null | bike Level of Traffic Stress (1 = kids OK, 4 = fast traffic); null if `bike` is false or unknown |
| `wheelchair` | string? | OSM `wheelchair` tag as-is |
| `near_bench_m`, `near_toilets_m`, `near_water_m` | f32, 0–500 | distance from the edge to the nearest bench / toilets / drinking water, **capped at 500** (500 = none nearby). Missing = 500 |
| `tree` | f32 0–1 / null | static tree-cover fraction over the edge. **Slot for later** (null now) |
| `shade` | `[f32; slots]` 0–1 / null | shade fraction per time slot (`meta.slots` values, slot i covers `[i·slot_min, (i+1)·slot_min)`). **Slot for the LiDAR task** (null now) |

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
| `category` | string | one of `bench`, `toilets`, `drinking_water`, `cafe`, `restaurant`, `park`, `viewpoint`, `library`, `pharmacy`, `shelter` (others allowed; the UI shows unknown ones as "other") |
| `tags` | object | selected OSM tags as-is (`opening_hours`, `wheelchair`, `website`, …) |
| `node` | u32 | nearest graph node (that is where `field` reads the place's cost) |
| `snap_m` | f32 | distance place → node |
| `rating`, `n`, `rating_src` | f32? / u32? / string? | rating (0–5), number of reviews, where it came from. null until task 005 joins ratings |

Place cost = cost of its snapped node (the `snap_m` walk is ignored for now; it is the same for every metric).

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
Current presets: walk, hiking (Naismith), wheelchair, stroller, shade, night, rest, rain, bike.
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

**Status:** `field` is a **stub** (straight-line distance / mode speed, ignores weights and time) until task 003
puts in petgraph Dijkstra over `Metric::edge_cost`. Signature and output stay as documented.

Native Rust (tests, a future server) uses the same logic through `engine::Core`
(`Core::load`, `nearest_node`, `field(origin, &Metric, t)`).

## 5. Fixture

`data/fixtures/graph.json` + `places.json`: a hand-made 4 × 3 grid (12 nodes, 17 edges, 6 places, ~100 m spacing)
near Palmer Square. It is **not real data**. It covers every edge field: a busy oneway-for-bikes primary road, stairs,
an unlit footway, a gravel path, a covered walkway with a full `shade` array, a sett (rough) pedestrian street,
and an edge with unknown surface/lit. Use it in unit tests and for web work before the real data lands.
