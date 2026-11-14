# prep: Princeton data

Builds the files the engine and web load. Run once; outputs are committed, so nobody else needs to.

```sh
bash prep/fetch.sh               # downloads into data/raw/ (gitignored, ~10 min, skips what's cached)
cargo run --release -p prep      # data/raw/ -> data/princeton/{graph,places}.json + stats.txt (seconds, offline)
```

Format: `docs/contracts.md`. prep fills the engine's own structs (`engine::graph`) and runs the
engine's loader on its output before writing, so the files always match the contract.

## Sources
- **Streets, paths, amenities:** OpenStreetMap via Overpass API (`prep/overpass.ql`), bbox
  W -74.690, S 40.330, E -74.630, N 40.370 (downtown, campus, down to Lake Carnegie). (c) OSM contributors, ODbL.
- **Elevation:** USGS 3DEP 1/3 arc-second DEM (~10 m) via OpenTopoData's public API (`ned10m`),
  sampled on a ~25 m grid and interpolated bilinearly.

## How each feature is made (prep/src/tags.rs, one rule per line)
- **Graph:** OSM ways that people can walk or ride, clipped to the box, split at every point
  shared by two ways. Self-loops dropped; only the largest connected piece is kept.
- **foot / bike:** defaults by `highway` (no bikes on footways/steps, no walking on trunk roads
  without a mapped sidewalk), then `access`, `foot`, `bicycle` tags override.
- **climb / descent / max_grade_pct:** elevation sampled every 5 m along the edge; climb = sum of
  rises; grade = steepest rise or fall over any 20 m stretch (shorter windows only add DEM noise).
  Bridges and tunnels ignore the ground below and go straight from end to end.
- **surface:** OSM `surface` grouped by bumpiness (paved / compacted / unpaved / rough). Untagged
  car roads are assumed paved; untagged paths stay `null`.
- **lit, covered, wheelchair, steps:** straight from tags (`covered`, `tunnel`, `indoor`, corridors count as covered).
- **busy:** a road's own traffic class (1 local, 2 collector, 3 arterial). A footway or cycleway
  gets the class of the busiest road within 15 m (sidewalks, crossings); 0 if none.
- **cycleway, lts:** simplified bike Level of Traffic Stress (Mekuria/Furth/Nixon 2012) from road
  class, bike lane/track, speed limit and lane count. Car-free paths are LTS 1.
- **near_bench_m / near_toilets_m / near_water_m:** distance from the edge to the nearest one, capped at 500 m.
- **places:** OSM amenities/tourism/leisure of the categories in `tags::place_category`; areas use
  their centroid and snap to the graph node nearest to their outline.
- **meta.sunrise_min / sunset_min:** NOAA solar formula for the demo date.
- **shade, tree:** `null`, filled by a later LiDAR / tree-cover task.

`data/princeton/stats.txt` lists counts and how much of the network carries each tag.
