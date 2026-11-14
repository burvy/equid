# Devlog

Short entries: challenge → what we did. Times are Saturday, from the git log.

- **03:50 · Too many "finders" to build (shade, night, stroller, bike…).** One engine instead: streets × features × your
  weights. Every finder is just a list of weights.
- **04:16 · Must feel live.** Rust engine compiled to WebAssembly; Dijkstra runs in the browser, so sliders redraw at once.
- **04:24 · "Close for you" is hard to show on a normal map.** Warp and 3D views: the map bends so distance = your cost.
- **04:43 · Real streets, not a toy.** Princeton walk/bike graph from OpenStreetMap + USGS elevation.
- **05:09 · Most streets don't say if they're lit.** Rules fill the gaps (lamps, road class, campus) and record which
  rule was used, so we never pass a guess off as a fact.
- **05:25 · "Shady route" needs real shade.** 227 M LiDAR points → roofs and trees → sun or shadow on every street,
  every half hour.
- **04:34–05:30 · Sliders are hard for a stressed user.** Claude turns plain English ("stroller, want a coffee") into
  weights and stops, then a check makes sure its pitch only claims what the engine computed.
- **04:50 · Can't map every bench and dumpster by hand.** Open-vocabulary objects: OSM points + Vexcel aerial detection.
  Vexcel refused Princeton (geofence), so Houston became the second region.
- **06:07–06:26 · Health risks beyond hills.** Flood zones (FEMA), crossings, missing sidewalks, raised kerbs, dull
  streets, icy shade: new features and presets (flood-safe, peaceful, few crossings).
- **06:57 · Routes dodged crossings by cutting through junctions.** Crossings are now charged at junctions from the
  street topology, so they can't be dodged.
- **07:04 · Users and the AI can make any metric; is Dijkstra still right?** Lean 4 proof: every feature ≥ 0, so any
  weights ≥ 0 work. The engine now runs Rust generated from the proven formulas.
- **07:22–07:41 · Where should I live?** Houston grew to 4×4 km (adds Buffalo Bayou); CoStar homes and apartments
  ranked by *your* metric from each front door, with a Matterport tour link. Listing data stays local (unknown terms).
