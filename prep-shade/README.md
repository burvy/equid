# prep-shade: LiDAR shade, tree cover and sunset spots

Fills `edges[].shade` (48 half-hour slots) and `edges[].tree` in `data/princeton/graph.json`, and adds
`sunset` places to `data/princeton/places.json`. Runs **after** `prep` (re-running `prep` wipes shade until this runs again).

```sh
cargo run --release -p prep                 # OSM + elevation -> graph.json, places.json
cargo run --release -p prep-shade           # LiDAR -> shade, tree, sunset spots (+ data/princeton/shade_stats.txt)
```

The first run downloads ~1.4 GB of LiDAR (2 min) and builds the rasters (40 s), caching both in
`data/raw/lidar/` (gitignored). After that the command is offline and takes ~2 s.
Steps can be run alone: `-- fetch`, `-- raster`, `-- shade`.

## The idea in one breath
A plane flew over Princeton firing a laser, which records the height of everything: roofs, tree crowns and the ground.
From that we know, for any spot on any street and any time of day, whether the line from your head to the sun hits a roof
(no sun), passes through a tree (some sun) or is clear (full sun).

## Step by step

**1. Fetch (`src/ept.rs`).** USGS 3DEP LiDAR, project `USGS_LPC_DE_DelawareValley_HD_2015` (flown 2015, public domain),
from the free Entwine Point Tile copy on AWS (`s3://usgs-lidar-public`, no key). EPT is an octree of LAZ files; we take every
tile touching the region box down to depth 10 (~4 points per m²), plus a 1 km ring at depth 9 so shadows and sunset
skylines coming from just outside the box are seen. 2,846 tiles, 227 M points.

**2. Raster (`src/raster.rs`).** Points → 1 m grids in a flat local frame (metres east/north of the box centre):
- **DSM** (surface): highest return per cell (roofs, crowns, ground).
- **DTM** (ground): lowest ground-classified return (LAS class 2); gaps under buildings are filled from the surrounding ground.
- **deck**: bridge-deck returns (class 17), so people on a bridge stand on it, not on the water below.
- **tree**: crown or roof? This survey left both "unclassified", so we decide per cell (only cells > 2.5 m above ground):
  a crown if ≥ 25 % of the laser pulses were split (leaves give several echoes, roofs one), or if the surface is rough
  (roofs are flat planes, crowns are bumpy). A 3×3 majority vote cleans roof edges.
- Clean-up: noise classes dropped, single-cell spikes (birds) flattened, anything > 70 m above ground discarded, and a
  3×3 morphological *opening* (min filter, then max filter) removes things narrower than ~3 m (overhead wires, poles) that
  would otherwise act like solid walls.
- Check: `data/raw/lidar/ndsm.png` (grey ground, green trees, orange buildings) and `raster_stats.txt`.

**3. Sun (`src/sun.rs`).** NOAA's solar position formulas (same as `prep` uses for sunrise/sunset) give the sun's altitude and
azimuth for the demo date (`meta.date`) at any minute, with refraction. One direction for the whole town (the sun is far away).

**4. Shade per edge (`src/shade.rs`, `src/edges.rs`).** Points every ~5 m along each edge, at head height (ground + 1.6 m).
For each point and each moment, walk from the point toward the sun over the 1 m grid (ray marching):
- roof, wall or hill above the ray: **blocked** (0 sun);
- tree crown above the ray: the light is **dimmed** by the Beer–Lambert law, `T = exp(-κ · metres of crown crossed)`.
  κ depends on the season: 0.30/m with full leaves (Jun–Sep), 0.05/m bare (Dec–Mar), linear in between. For Nov 15, κ ≈ 0.11/m:
  a beam through 10 m of crown keeps ~33 %. A ray that passes *under* a crown (below 35 % of the tree's height, at least 2.5 m) is not dimmed.

`shade[slot] = 1 − average sunlight` over the edge's points at 3 moments in the slot (+5, +15, +25 min). Night slots are 1.
Covered edges (arcades, tunnels) are 1 all day. Speed-up: the grid is cut into 32 m blocks with their max height; while the
ray is above a block's top it jumps over the block (exact, the ray only climbs).

**5. Tree (`edges.rs`).** `tree` = share of the cells within ±2 m of the edge's points that are tree crown (static, 0–1).

**6. Sunset spots (`src/sunset.rs`).** Every graph node is a candidate. For each minute of the sun's last 90 minutes, the sun is
*visible* if ≥ 50 % of its light reaches the point (same ray test). `sun_visible_until` = the last visible minute. Spots must have
an open view toward the sunset (nothing more than 3° above the horizon in that direction) and ≥ 1.2 km of LiDAR in that direction
(beyond the data we assume an open horizon). The 8 latest ones, ≥ 400 m apart, become places with category `sunset` and tags
`sun_visible_until` ("16:35"), `sun_visible_until_min`, `sunset`, `skyline_deg`, `date`, `source`. They are named after the
nearest named park (≤ 200 m) or street (≤ 150 m). Existing parks/gardens/viewpoints get a `sun_visible_until` tag too.

**7. Check and write.** The engine's own loader (`Core::load`) must accept the files before they are written.
`data/princeton/shade_stats.txt` holds the stats, a named-street sanity table and a 13:00 vs 16:00 route demo (`src/demo.rs`).

## Results (Nov 15, 2026)
- Share of the network in shade: 49 % at 12:00, 53 % at 13:00, 62 % at 14:00, 96 % at 16:00 (sun at 4°).
- Sanity: wooded Pretty Brook Rd 92 % shaded at 13:00, Hodge Rd 72 %; open US-1 (Brunswick Pike) 1 %, the cross-country course 0 %.
- Route demo, shade preset (time + 1 s per sunny metre), Princeton Battlefield → Lake Carnegie boathouse: the walk route has
  2,425 sunny metres at 13:00. The 13:00 shade route walks 485 m more and has 1,512 sunny metres. The 16:00 shade route is a
  different one (only 5 % of nodes shared) with 43 sunny metres.
- Sunset 16:43; best spot keeps the sun until 16:35 (Rosedale Road fields). Nobody keeps it to the last minute: the
  horizon is lined with trees ~0.5–1° high.

## Limits (say them before a judge does)
- LiDAR is from 2015 (newer buildings and grown trees are missing) and was flown leaf-off; crown density is modelled by season, not measured.
- κ, the crown base and the 50 % "visible" threshold are round, defensible numbers, not calibrated against light meters.
- Shade is computed on the street centreline, so both sidewalks share one value. The whole trip is costed at the departure time (engine rule).
- Sunset skylines stop at the LiDAR edge (1 km beyond the box); farther terrain is assumed flat (true for the farmland west of Princeton).
