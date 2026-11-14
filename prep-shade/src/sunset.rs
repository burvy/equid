//! Sunset viewpoints: places on the walk network from which you can watch the sun go down.
//!
//! For a point (at head height), take every minute of the sun's last 90 minutes. The sun counts as
//! visible in that minute if at least half of its direct light reaches the point (same ray test as the
//! shade: roofs and hills block, bare tree crowns dim). "Sun visible until" = the last visible minute.
//! A good sunset spot keeps the sun until close to the official sunset.

use crate::geo::Proj;
use crate::shade::{Scene, EYE_M};
use crate::sun::Day;
use rayon::prelude::*;

/// Need at least this much LiDAR between the point and the edge of the data toward the sunset,
/// otherwise we cannot vouch for the view (past the data the horizon is assumed open).
const MIN_SEEN_M: f64 = 1200.0;
/// Visible = at least this share of direct sunlight gets through.
const VISIBLE: f64 = 0.5;

pub struct Candidate {
    pub x: f64,
    pub y: f64,
}

pub struct Verdict {
    /// Last minute (local, minutes since midnight) the sun is visible; None = not in the last 90 minutes.
    pub visible_until: Option<u32>,
    /// Skyline angle (deg) toward the sun at sunset: the steepest roof/crown/hill in that direction.
    pub skyline_deg: f64,
    /// Was there enough LiDAR toward the sun to trust this?
    pub trusted: bool,
}

pub fn evaluate(scene: &Scene, proj: &Proj, day: &Day, sunset_min: f64, cands: &[Candidate]) -> Vec<Verdict> {
    let end = sunset_min.floor() as u32;
    let minutes: Vec<(u32, f64, f64)> = (end.saturating_sub(90)..=end)
        .map(|t| {
            let (alt, az) = day.sun(t as f64, proj.lat0, proj.lon0);
            (t, alt, az)
        })
        .collect();
    let az_set = minutes.last().map(|m| m.2).unwrap_or(245.0);
    let g = &scene.dsm;
    let (gx1, gy1) = (g.x0 + g.nx as f64 * g.cell, g.y0 + g.ny as f64 * g.cell);
    cands
        .par_iter()
        .map(|c| {
            let Some(ground) = scene.ground(c.x, c.y) else {
                return Verdict { visible_until: None, skyline_deg: 90.0, trusted: false };
            };
            let z0 = ground + EYE_M;
            let until = minutes.iter().rev().find(|&&(_, alt, az)| scene.sunlight(c.x, c.y, z0, alt, az) >= VISIBLE).map(|m| m.0);
            let (h, _) = scene.horizon_tan(c.x, c.y, z0, az_set, 5000.0, f64::INFINITY);
            // distance from the point to the edge of the data, looking toward the sunset
            let (dx, dy) = (az_set.to_radians().sin(), az_set.to_radians().cos());
            let tx = if dx > 0.0 { (gx1 - c.x) / dx } else if dx < 0.0 { (g.x0 - c.x) / dx } else { f64::INFINITY };
            let ty = if dy > 0.0 { (gy1 - c.y) / dy } else if dy < 0.0 { (g.y0 - c.y) / dy } else { f64::INFINITY };
            Verdict { visible_until: until, skyline_deg: h.atan().to_degrees(), trusted: tx.min(ty) >= MIN_SEEN_M }
        })
        .collect()
}
