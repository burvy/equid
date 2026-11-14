#!/usr/bin/env bash
# Downloads raw inputs into data/raw/ (gitignored). Safe to re-run: skips files that exist.
# 1) OSM ways + amenities from Overpass (prep/overpass.ql).
# 2) Elevation on a ~25 m grid from OpenTopoData's public API, dataset ned10m
#    (USGS 3DEP 1/3 arc-second, ~10 m), 100 points per request, ~1 request/second
#    (the public API's limit; 1000 requests/day, this needs ~370).
set -euo pipefail
cd "$(dirname "$0")/.."
RAW=data/raw; mkdir -p "$RAW/ned10m"
UA="equid-prep/0.1 (hackathon data prep)"
# bbox and grid step: must match GRID in prep/src/main.rs
S=40.330; W=-74.690; N=40.370; E=-74.630
DLAT=0.000225; DLON=0.000295

if [ ! -s "$RAW/overpass.json" ]; then
  echo "overpass..."
  curl -sf -A "$UA" --data-urlencode "data@prep/overpass.ql" -o "$RAW/overpass.json" https://overpass-api.de/api/interpreter
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
