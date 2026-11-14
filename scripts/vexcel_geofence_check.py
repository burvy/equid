"""Reproduce the Vexcel geofence error: one small elements/detect call over
Princeton and the same call over Houston (the tutorial's point).

Usage:  python scripts/vexcel_geofence_check.py [path/to/vexcel.env]
Key: VEXCEL_API_KEY env var, else the env file (VEXCEL_API_KEY=... or bare key).
The key is only sent in the X-API-KEY header; it is never printed.
Standard library only.
"""
import json, os, sys, urllib.error, urllib.request

API = "https://api.vexcelgroup.com/v2/elements/detect"
ENV_FILE = sys.argv[1] if len(sys.argv) > 1 else r"Z:\App Data\Downloads\vexcel.env"
SITES = {  # (lon, lat) centres; boxes are ~150 m per side
    "Princeton, NJ (Nassau St)": (-74.6590, 40.3500),
    "Houston, TX (tutorial point)": (-95.4100, 29.7500),
}

def load_key():
    k = os.environ.get("VEXCEL_API_KEY", "").strip()
    if k:
        return k
    lines = [l.strip() for l in open(ENV_FILE, encoding="utf-8") if l.strip() and not l.startswith("#")]
    for l in lines:
        l = l.removeprefix("export ").strip()
        if l.startswith("VEXCEL_API_KEY") and "=" in l:
            return l.split("=", 1)[1].strip().strip("'\"")
    if len(lines) == 1 and "=" not in lines[0]:
        return lines[0]
    sys.exit("no VEXCEL_API_KEY found")

def detect(key, lon, lat, label="bench", half=0.0007):
    w, s, e, n = lon - half, lat - half * 0.76, lon + half, lat + half * 0.76
    wkt = f"POLYGON(({w} {s},{e} {s},{e} {n},{w} {n},{w} {s}))"  # WKT is lon lat
    body = {
        "wkt": wkt, "srid": 4326, "buffer-meter": 0,
        "images": [{"endpoint": "ortho/extract",
                    "request": {"layer": "urban,urban-r,wide-area,urban-g", "srid": 4326, "wkt": wkt, "crop": "bounding_box"}}],
        "model-input": {"elements": [{"label": label, "model": "sam3", "detection-threshold": 0.5}],
                        "include": ["bbox", "confidence"]},
    }
    req = urllib.request.Request(API, data=json.dumps(body).encode(), method="POST",
                                 headers={"X-API-KEY": key, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=300) as r:
            v = json.load(r)
            hits = sum(len(f.get("properties", {}).get("detections", [])) for f in v.get("features", []))
            return f"HTTP {r.status} OK: {hits} '{label}' detections"
    except urllib.error.HTTPError as err:
        return f"HTTP {err.code}: {err.read().decode(errors='replace')[:300]}"

if __name__ == "__main__":
    key = load_key()
    for name, (lon, lat) in SITES.items():
        print(f"{name} @ {lat},{lon}\n  {detect(key, lon, lat)}")
