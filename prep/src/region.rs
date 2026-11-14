//! Which area to build: one line of prep/regions.tsv (also read by prep/fetch.sh).

use std::path::Path;

pub struct Region {
    pub name: String,
    /// [west, south, east, north]
    pub bbox: [f64; 4],
    pub tz: String,
    /// Offset from UTC on the demo day (for sunrise/sunset).
    pub utc_offset_h: f64,
    /// University polygons whose name starts with this count as lit campus (night.rs).
    pub campus_prefix: Option<String>,
    /// Download cache, relative to the repo root.
    pub raw_dir: String,
}

impl Region {
    pub fn load(path: &Path, name: &str) -> Result<Region, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut known = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() != 9 {
                return Err(format!("regions.tsv: expected 9 fields: {line}"));
            }
            known.push(f[0].to_string());
            if f[0] != name {
                continue;
            }
            let num = |s: &str| s.parse::<f64>().map_err(|e| format!("regions.tsv: {s}: {e}"));
            return Ok(Region {
                name: f[0].into(),
                bbox: [num(f[1])?, num(f[2])?, num(f[3])?, num(f[4])?],
                tz: f[5].into(),
                utc_offset_h: num(f[6])?,
                campus_prefix: (f[7] != "-").then(|| f[7].to_string()),
                raw_dir: f[8].into(),
            });
        }
        Err(format!("unknown region '{name}' (prep/regions.tsv has: {})", known.join(", ")))
    }
}
