# Devpost draft: Equid

**Tagline:** A map for how you are right now.
**Track:** Healthcare (pending ask 043) · **Repo:** https://github.com/burvy/equid
**Team:** TODO names + Discord handles (ask 045)

## Inspiration
A parent with a stroller, someone in a wheelchair, a runner at night, anyone on a 35 °C afternoon: the "5-minute walk"
on a normal map is a lie for them. Distance isn't just metres. It's hills, steps, shade, light, kerbs and flood water.

## What it does
- You say how you are (sliders, a preset like *wheelchair* or *shady*, or plain English to the AI).
- Equid computes your personal cost to every street and place, and **bends the map**: in the warp view, distance
  from you = your cost. Places that are close *for you* come close.
- The AI trip agent (Claude) turns "stroller, want a coffee before 10" into weights and stops, and only says what
  the engine computed.
- Catch the sunset: which viewpoints you can still reach before the sun leaves them.
- Houston: rank homes and apartments by **your** metric from each front door, with a Matterport 3D tour.

## How we built it
- **Engine:** Rust → WebAssembly. 20 street features × your weights; Dijkstra (petgraph) in the browser.
- **Data:** OpenStreetMap, USGS elevation, 227 M points of USGS LiDAR for shade and trees every half hour,
  FEMA flood zones, Vexcel aerial detections, CoStar listings.
- **Web:** deck.gl + MapLibre, Vite + TypeScript.
- **AI:** Claude tool use over the engine (make_metric, field, list_places, detect, rank_listings, …); a check keeps its pitch to what the engine computed.
- **Proof:** Lean 4 proves every feature ≥ 0, so any weights ≥ 0 give a valid Dijkstra metric. The engine runs Rust
  generated from the proven formulas.

## Challenges
- Most streets don't say if they're lit or have a sidewalk: we infer with labelled rules and show coverage honestly.
- Vexcel's imagery was geofenced off Princeton, so we added Houston as a second region.
- Routes dodged crossings by cutting through junctions until we charged crossings from the street topology.

## Accomplishments
- One engine, many needs: every "finder" is just a list of weights.
- Real LiDAR shade on every street, every half hour.
- A machine-checked guarantee that any metric a user or the AI makes is safe for Dijkstra.

## What we learned
Honest data beats more data: saying "unknown" (and how much is unknown) keeps the map trustworthy.

## What's next
More regions, live weather (ice, heat), crowd-sourced kerb and bench reports, and transit.

## Built with
rust · webassembly · petgraph · deck.gl · maplibre · typescript · vite · claude · anthropic-api · lean4 · axum ·
openstreetmap · usgs-3dep · lidar · fema-nfhl · vexcel · costar · matterport
