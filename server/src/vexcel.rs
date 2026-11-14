//! Vexcel `elements/detect` client: plain-language label + small area -> object locations.
//! The API key is read from `VEXCEL_API_KEY`, or from a `VEXCEL_API_KEY=...` line in the env file named by
//! `VEXCEL_ENV_FILE` (default `Z:\App Data\Downloads\vexcel.env`). It is only ever sent as the `X-API-KEY`
//! header: never printed, logged, put in errors or committed.

use serde_json::{json, Value};

const API: &str = "https://api.vexcelgroup.com/v2";
const LAYERS: &str = "urban,urban-r,wide-area,urban-g";
pub const DEFAULT_ENV_FILE: &str = r"Z:\App Data\Downloads\vexcel.env";
/// Largest area (metres per side) a live detect may cover: detection is seconds per call, keep it small.
pub const MAX_SIDE_M: f64 = 600.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub lon: f64,
    pub lat: f64,
    pub conf: f32,
}

pub enum Detector {
    Live { key: String, client: reqwest::Client },
    /// Canned answer (offline demo / tests): `n` hits scattered around the area centre.
    Fixed { n: usize },
}

/// Reads the key from `VEXCEL_API_KEY=...` (optionally `export`ed / quoted), or, failing that, a file that is just the bare key.
pub fn key_from_env_file(path: &str, var: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let named = lines.iter().find_map(|l| {
        let l = l.trim_start_matches("export ").trim();
        let v = l.strip_prefix(var)?.trim_start().strip_prefix('=')?;
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        (!v.is_empty()).then(|| v.to_string())
    });
    named.or_else(|| match lines.as_slice() {
        [only] if !only.contains(char::is_whitespace) && !only.contains('=') => Some(only.to_string()),
        _ => None,
    })
}

impl Detector {
    /// Live detector if a key is found, else None.
    pub fn from_env() -> Option<Detector> {
        let key = std::env::var("VEXCEL_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .map(|k| k.trim().to_string())
            .or_else(|| key_from_env_file(&std::env::var("VEXCEL_ENV_FILE").unwrap_or_else(|_| DEFAULT_ENV_FILE.to_string()), "VEXCEL_API_KEY"))?;
        Some(Detector::Live { key, client: reqwest::Client::new() })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Detector::Live { .. } => "vexcel",
            Detector::Fixed { .. } => "fixed",
        }
    }

    /// Detect one `label` inside `bbox` = [west, south, east, north] (degrees).
    pub async fn detect(&self, label: &str, bbox: [f64; 4]) -> Result<Vec<Detection>, String> {
        Ok(self.detect_many(&[label.to_string()], bbox).await?.into_iter().next().map(|x| x.1).unwrap_or_default())
    }

    /// Several labels in one call (one imagery fetch): `(label, detections)` per requested label, in order.
    pub async fn detect_many(&self, labels: &[String], bbox: [f64; 4]) -> Result<Vec<(String, Vec<Detection>)>, String> {
        let [w, s, e, n] = bbox;
        let side = ((e - w) * 111_320.0 * ((s + n) / 2.0).to_radians().cos()).max((n - s) * 110_540.0);
        if side > MAX_SIDE_M {
            return Err(format!("area too large for a live detect ({side:.0} m > {MAX_SIDE_M:.0} m per side)"));
        }
        match self {
            Detector::Fixed { n: count } => Ok(labels
                .iter()
                .map(|l| {
                    let hits = (0..*count)
                        .map(|i| {
                            let f = (i as f64 + 1.0) / (*count as f64 + 1.0);
                            Detection { lon: w + (e - w) * f, lat: s + (n - s) * (1.0 - f), conf: 0.9 }
                        })
                        .collect();
                    (l.clone(), hits)
                })
                .collect()),
            Detector::Live { key, client } => {
                let wkt = format!("POLYGON(({w} {s},{e} {s},{e} {n},{w} {n},{w} {s}))");
                let body = json!({
                    "wkt": wkt, "srid": 4326, "buffer-meter": 0,
                    "images": [{"endpoint": "ortho/extract",
                                "request": {"layer": LAYERS, "srid": 4326, "wkt": wkt, "crop": "bounding_box"}}],
                    "model-input": {
                        "elements": labels.iter().map(|l| json!({"label": l, "model": "sam3", "detection-threshold": 0.5})).collect::<Vec<_>>(),
                        "include": ["bbox", "confidence"],
                    },
                });
                let v = post_with_backoff(client, key, "elements/detect", &body).await?;
                let all = parse_detections(&v);
                Ok(labels.iter().map(|l| (l.clone(), all.iter().filter(|d| &d.label == l).map(|d| d.det.clone()).collect())).collect())
            }
        }
    }
}

async fn post_with_backoff(client: &reqwest::Client, key: &str, path: &str, body: &Value) -> Result<Value, String> {
    for attempt in 0..=5u32 {
        let r = client
            .post(format!("{API}/{path}"))
            .header("X-API-KEY", key)
            .json(body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| format!("vexcel request failed: {}", e.without_url()))?;
        if r.status().as_u16() == 429 && attempt < 5 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            continue;
        }
        let status = r.status();
        let text = r.text().await.map_err(|e| format!("vexcel response: {}", e.without_url()))?;
        if !status.is_success() {
            return Err(format!("vexcel {status}: {}", text.chars().take(300).collect::<String>()));
        }
        return serde_json::from_str(&text).map_err(|e| format!("vexcel response not JSON: {e}"));
    }
    Err("vexcel: rate limited".into())
}

/// Centre of a WKT `POLYGON((x y, x y, ...))` as (lon, lat).
pub fn wkt_centre(wkt: &str) -> Option<(f64, f64)> {
    let inner = wkt.split("((").nth(1)?.split("))").next()?;
    let pts: Vec<(f64, f64)> = inner
        .split(',')
        .filter_map(|p| {
            let mut it = p.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    if pts.is_empty() {
        return None;
    }
    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
    Some(((x0 + x1) / 2.0, (y0 + y1) / 2.0))
}

pub struct Labelled {
    pub label: String,
    pub det: Detection,
}

/// `features[].properties.detections[] {bbox: WKT polygon, confidence}` -> labelled centres.
pub fn parse_detections(v: &Value) -> Vec<Labelled> {
    let mut out = vec![];
    for f in v["features"].as_array().into_iter().flatten() {
        for d in f["properties"]["detections"].as_array().into_iter().flatten() {
            if let Some((lon, lat)) = d["bbox"].as_str().and_then(wkt_centre) {
                let label = f["properties"]["label"].as_str().unwrap_or("").to_string();
                out.push(Labelled { label, det: Detection { lon, lat, conf: d["confidence"].as_f64().unwrap_or(0.0) as f32 } });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_detect_response_shape() {
        let v = json!({"features": [{"properties": {"label": "car", "detections": [
            {"imageId": "a", "confidence": 0.8, "bbox": "POLYGON((-74.66 40.34,-74.658 40.34,-74.658 40.342,-74.66 40.342,-74.66 40.34))"}]}}]});
        let d = parse_detections(&v);
        assert_eq!(d.len(), 1);
        assert!((d[0].det.lon + 74.659).abs() < 1e-9 && (d[0].det.lat - 40.341).abs() < 1e-9 && d[0].det.conf == 0.8 && d[0].label == "car");
    }

    #[test]
    fn env_file_line_parsing_never_needs_the_real_key() {
        let dir = std::env::temp_dir().join("equid_vexcel_test.env");
        std::fs::write(&dir, "# c\nOTHER=1\nexport VEXCEL_API_KEY = \"abc123\"\n").unwrap();
        assert_eq!(key_from_env_file(dir.to_str().unwrap(), "VEXCEL_API_KEY").as_deref(), Some("abc123"));
        std::fs::write(&dir, "  barekeyvalue123  
").unwrap();
        assert_eq!(key_from_env_file(dir.to_str().unwrap(), "VEXCEL_API_KEY").as_deref(), Some("barekeyvalue123"));
        std::fs::remove_file(dir).ok();
    }
}
