#!/usr/bin/env bash
# Downloads raw inputs for one region (prep/regions.tsv) into its raw_dir (gitignored).
# Safe to re-run: skips files that exist.
#   bash prep/fetch.sh            # princeton (default) -> data/raw/
#   bash prep/fetch.sh houston    # -> data/raw/houston/
# 1) OSM ways + amenities (prep/overpass.ql), street lamps + campus outlines (prep/night.ql), point objects
#    (prep/objects.ql), crossing/signal/stop/kerb nodes (prep/nodes.ql), green + water areas (prep/landscape.ql),
#    all from Overpass; the queries' {{BBOX}} is filled from regions.tsv. Then FEMA NFHL flood zones.
# 2) Elevation on a ~25 m grid from OpenTopoData's public API, dataset ned10m
#    (USGS 3DEP 1/3 arc-second, ~10 m), 100 points per request, ~1 request/second
#    (the public API's limit; 1000 requests/day; Princeton needs ~370, Houston ~60).
set -euo pipefail
cd "$(dirname "$0")/.."
REGION=${1:-princeton}
read -r W S E N RAW < <(awk -v r="$REGION" '$1==r {print $2, $3, $4, $5, $9}' prep/regions.tsv)
[ -n "${RAW:-}" ] || { echo "unknown region '$REGION' (see prep/regions.tsv)"; exit 1; }
mkdir -p "$RAW/ned10m"
UA="equid-prep/0.1 (hackathon data prep)"
# grid step (~25 m)
DLAT=0.000225; DLON=0.000295

overpass() { # $1 = query file, $2 = output
  [ -s "$2" ] && return 0
  echo "overpass $1..."
  sed "s/{{BBOX}}/$S,$W,$N,$E/g" "$1" > "$2.ql"
  for try in 1 2 3 4 5; do
    curl -sf -A "$UA" --data-urlencode "data@$2.ql" -o "$2.tmp" https://overpass-api.de/api/interpreter && mv "$2.tmp" "$2" && break
    echo "  failed (try $try; Overpass rate limit?), backing off"; sleep $((try*15))
  done
  rm -f "$2.ql" "$2.tmp"
  [ -s "$2" ] || { echo "giving up on $1"; exit 1; }
  sleep 5
}
overpass prep/overpass.ql "$RAW/overpass.json"
overpass prep/night.ql "$RAW/night.json"
overpass prep/objects.ql "$RAW/objects.json"
overpass prep/nodes.ql "$RAW/nodes.json"
overpass prep/landscape.ql "$RAW/landscape.json"

# FEMA National Flood Hazard Layer, layer 28 (flood hazard zones): only the 1% (SFHA) and 0.2% zones, GeoJSON.
if [ ! -s "$RAW/flood.json" ]; then
  echo "FEMA NFHL flood zones..."
  curl -sf -m 180 -A "$UA" -G https://hazards.fema.gov/arcgis/rest/services/public/NFHL/MapServer/28/query     --data-urlencode "geometry=$W,$S,$E,$N" --data-urlencode "geometryType=esriGeometryEnvelope"     --data-urlencode "inSR=4326" --data-urlencode "outSR=4326" --data-urlencode "spatialRel=esriSpatialRelIntersects"     --data-urlencode "where=SFHA_TF='T' OR ZONE_SUBTY LIKE '0.2 PCT%'"     --data-urlencode "outFields=FLD_ZONE,ZONE_SUBTY,SFHA_TF" --data-urlencode "geometryPrecision=6"     --data-urlencode "f=geojson" -o "$RAW/flood.json.tmp"
  grep -q '"features"' "$RAW/flood.json.tmp" && mv "$RAW/flood.json.tmp" "$RAW/flood.json" || { echo "NFHL query failed"; exit 1; }
fi

# grid points, row-major: rows from S to N, columns from W to E
awk -v s=$S -v w=$W -v n=$N -v e=$E -v dl=$DLAT -v dn=$DLON 'BEGIN{
  rows=int((n-s)/dl)+2; cols=int((e-w)/dn)+2;
  for(i=0;i<rows;i++) for(j=0;j<cols;j++) printf "%.6f,%.6f\n", s+i*dl, w+j*dn }' > "$RAW/elev_points.txt"
total=$(wc -l < "$RAW/elev_points.txt")
batches=$(( (total + 99) / 100 ))
echo "elevation: $total points, $batches requests"
for ((b=0; b<batches; b++)); do
  f=$(printf "%s/ned10m/%04d.json" "$RAW" $b)
  [ -s "$f" ] && continue
  locs=$(sed -n "$((b*100+1)),$((b*100+100))p" "$RAW/elev_points.txt" | paste -sd'|' -)
  for try in 1 2 3 4 5 6; do
    if curl -sf -A "$UA" -X POST -H "Content-Type: application/json" \
        -d "{\"locations\":\"$locs\",\"interpolation\":\"bilinear\"}" -o "$f.tmp" \
        https://api.opentopodata.org/v1/ned10m && grep -q '"OK"' "$f.tmp"; then
      mv "$f.tmp" "$f"; break
    fi
    echo "  batch $b failed (try $try), backing off"; sleep $((try*5))
  done
  [ -s "$f" ] || { echo "giving up on batch $b"; exit 1; }
  sleep 1.1
done
echo "done"
