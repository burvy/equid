//! Raster step: LiDAR points -> 1 m grids in the local frame.
//!
//! - `dsm` (digital surface model): highest return in each cell = top of roofs, tree crowns, ground.
//! - `dtm` (digital terrain model): lowest ground-classified return (LAS class 2); holes (under
//!   buildings, water) are filled from the surrounding ground.
//! - `deck`: highest bridge-deck return (class 17), NaN elsewhere, so people on a bridge stand on it.
//! - `tree`: 1 where the surface is a tree crown, 0 elsewhere (see `tree_mask`).
//!
//! This survey (2015) only classified ground, water, bridges and noise: roofs and trees are both
//! "unclassified". For shade that does not matter (both block the sun); for tree cover we tell
//! them apart ourselves.
//!
//! Noise: classes 7/18 and withheld points are dropped; single-cell spikes (birds, wires) are flattened,
//! and anything more than 70 m above ground (Princeton's tallest tower is ~53 m) is discarded.

use crate::geo::{merc_to_lonlat, Grid, Proj};
use rayon::prelude::*;
use std::path::Path;

/// Tallest believable thing above the ground, metres.
const MAX_HEIGHT_M: f32 = 70.0;

pub struct Rasters {
    pub dsm: Grid,
    pub dtm: Grid,
    pub deck: Grid,
    pub tree: Grid,
}

/// One point binned into the grid.
struct P {
    k: u32,
    z: f32,
    class: u8,
    /// not the last return of its laser pulse: the pulse was split, typical of leaves and branches
    split: bool,
}

fn read_tile(path: &Path, proj: &Proj, g: &Grid, hist: &mut [u64; 256]) -> Result<Vec<P>, String> {
    let mut r = las::Reader::from_path(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::with_capacity(r.header().number_of_points() as usize);
    for p in r.points() {
        let p = p.map_err(|e| format!("{}: {e}", path.display()))?;
        let c = u8::from(p.classification);
        hist[c as usize] += 1;
        if c == 7 || c == 18 || p.is_withheld {
            continue;
        }
        let (lon, lat) = merc_to_lonlat(p.x, p.y);
        let (x, y) = proj.xy(lon, lat);
        let Some(k) = g.idx(x, y) else { continue };
        out.push(P { k: k as u32, z: p.z as f32, class: c, split: p.return_number < p.number_of_returns });
    }
    Ok(out)
}

pub fn build(tiles_dir: &Path, proj: &Proj, extent: [f64; 4], cell: f64) -> Result<(Rasters, String), String> {
    let list = std::fs::read_to_string(tiles_dir.join("tiles.txt")).map_err(|e| format!("tiles.txt: {e} (run the fetch step)"))?;
    let keys: Vec<&str> = list.lines().filter_map(|l| l.split_whitespace().next()).collect();
    let (x0, y0) = proj.xy(extent[0], extent[1]);
    let (x1, y1) = proj.xy(extent[2], extent[3]);
    let (nx, ny) = (((x1 - x0) / cell).ceil() as usize, ((y1 - y0) / cell).ceil() as usize);
    let mut dsm = Grid::new(x0, y0, cell, nx, ny, f32::NAN);
    let mut dtm = dsm.same_shape(f32::NAN);
    let mut deck = dsm.same_shape(f32::NAN);
    let mut n_high = vec![0u16; nx * ny]; // returns that are not ground/water/bridge
    let mut n_split = vec![0u16; nx * ny]; // ... of which split pulses
    let mut hist = [0u64; 256];
    let mut kept = 0u64;

    // Decode tiles in parallel batches; merge each batch into the grids on one thread (bounded memory).
    for batch in keys.chunks(64) {
        let parts: Vec<Result<(Vec<P>, [u64; 256]), String>> = batch
            .par_iter()
            .map(|k| {
                let mut h = [0u64; 256];
                read_tile(&tiles_dir.join(format!("{k}.laz")), proj, &dsm, &mut h).map(|v| (v, h))
            })
            .collect();
        for part in parts {
            let (pts, h) = part?;
            for (a, b) in hist.iter_mut().zip(h.iter()) {
                *a += b;
            }
            kept += pts.len() as u64;
            for p in pts {
                let k = p.k as usize;
                if !(dsm.v[k] >= p.z) {
                    dsm.v[k] = p.z; // max, NaN-aware
                }
                match p.class {
                    2 => {
                        if !(dtm.v[k] <= p.z) {
                            dtm.v[k] = p.z; // min
                        }
                    }
                    17 => {
                        if !(deck.v[k] >= p.z) {
                            deck.v[k] = p.z;
                        }
                    }
                    9 => {}
                    _ => {
                        n_high[k] = n_high[k].saturating_add(1);
                        if p.split {
                            n_split[k] = n_split[k].saturating_add(1);
                        }
                    }
                }
            }
        }
    }
    let empty_dsm = dsm.v.iter().filter(|v| v.is_nan()).count();
    let empty_dtm = dtm.v.iter().filter(|v| v.is_nan()).count();
    dtm.fill_holes();
    // Implausibly high returns -> no data; then cells with no return (scan gaps, water) are filled
    // with the average of their neighbours, and the surface is never below the ground.
    let mut too_high = 0usize;
    for (s, t) in dsm.v.iter_mut().zip(dtm.v.iter()) {
        if *s - *t > MAX_HEIGHT_M {
            *s = f32::NAN;
            too_high += 1;
        }
    }
    dsm.fill_holes();
    for (s, t) in dsm.v.iter_mut().zip(dtm.v.iter()) {
        if *s < *t {
            *s = *t;
        }
    }
    let spikes = despike(&mut dsm, 3.0);
    open3(&mut dsm);
    for (s, t) in dsm.v.iter_mut().zip(dtm.v.iter()) {
        if *s < *t {
            *s = *t;
        }
    }
    let tree = tree_mask(&dsm, &dtm, &n_high, &n_split);

    let cells = (nx * ny) as f64;
    let total: u64 = hist.iter().sum();
    let classes: Vec<String> = hist.iter().enumerate().filter(|(_, n)| **n > 0).map(|(c, n)| format!("{c}:{:.1}%", *n as f64 * 100.0 / total as f64)).collect();
    let high_cells = dsm.v.iter().zip(dtm.v.iter()).filter(|(s, t)| **s - **t > 2.5).count();
    let tree_cells = tree.v.iter().filter(|v| **v > 0.5).count();
    let stats = format!(
        "raster: {} tiles, {:.1} M points in the grid, {nx} x {ny} cells of {cell} m\n  \
         classes (LAS code:share): {} (1 unclassified, 2 ground, 7/18 noise, 9 water, 17 bridge deck)\n  \
         cells with no return {:.1}%, no ground return {:.1}% (filled from neighbours)\n  \
         dropped: {too_high} cells > {MAX_HEIGHT_M} m above ground, {spikes} single-cell spikes\n  \
         ground {:.1}..{:.1} m, surface max {:.1} m\n  \
         cells > 2.5 m above ground: {:.1}% (of which tree crowns {:.1}%, the rest roofs etc.)\n",
        keys.len(),
        kept as f64 / 1e6,
        classes.join(" "),
        empty_dsm as f64 * 100.0 / cells,
        empty_dtm as f64 * 100.0 / cells,
        dtm.v.iter().cloned().fold(f32::INFINITY, f32::min),
        dtm.v.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
        dsm.v.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
        high_cells as f64 * 100.0 / cells,
        tree_cells as f64 * 100.0 / high_cells.max(1) as f64,
    );
    Ok((Rasters { dsm, dtm, deck, tree }, stats))
}

/// A cell more than `tol` metres above ALL eight neighbours is a stray return (bird, wire, antenna):
/// lower it to its highest neighbour. Returns how many cells changed.
fn despike(g: &mut Grid, tol: f32) -> usize {
    let (nx, ny) = (g.nx, g.ny);
    let src = g.v.clone();
    let mut n = 0;
    for j in 1..ny - 1 {
        for i in 1..nx - 1 {
            let k = j * nx + i;
            let mut m = f32::NEG_INFINITY;
            for dj in [-1i64, 0, 1] {
                for di in [-1i64, 0, 1] {
                    if di != 0 || dj != 0 {
                        m = m.max(src[(j as i64 + dj) as usize * nx + (i as i64 + di) as usize]);
                    }
                }
            }
            if src[k] > m + tol {
                g.v[k] = m;
                n += 1;
            }
        }
    }
    n
}

/// Tree crown vs roof, per cell more than 2.5 m above ground. Two clues, either is enough:
/// - split pulses: >= 25% of the cell's returns were not the last return of their pulse
///   (a laser pulse passing through leaves/branches returns several echoes; a roof returns one);
/// - roughness: crowns are bumpy, roofs are planes. Roughness = mean |height - neighbour height| in a
///   3 x 3 window after removing the local slope; > 0.5 m reads as a crown.
/// The mask is then cleaned with a 3 x 3 majority vote so roof edges don't count as trees.
fn tree_mask(dsm: &Grid, dtm: &Grid, n_high: &[u16], n_split: &[u16]) -> Grid {
    let (nx, ny) = (dsm.nx, dsm.ny);
    let mut raw = vec![0u8; nx * ny];
    raw.par_chunks_mut(nx).enumerate().for_each(|(j, row)| {
        if j == 0 || j == ny - 1 {
            return;
        }
        for i in 1..nx - 1 {
            let k = j * nx + i;
            if dsm.v[k] - dtm.v[k] <= 2.5 {
                continue;
            }
            let split = n_high[k] > 0 && n_split[k] as f32 >= 0.25 * n_high[k] as f32;
            let z = |di: i64, dj: i64| dsm.v[(j as i64 + dj) as usize * nx + (i as i64 + di) as usize];
            // plane through the centre with the central-difference slopes
            let (sx, sy) = ((z(1, 0) - z(-1, 0)) / 2.0, (z(0, 1) - z(0, -1)) / 2.0);
            let mut rough = 0.0;
            for dj in [-1i64, 0, 1] {
                for di in [-1i64, 0, 1] {
                    rough += (z(di, dj) - (z(0, 0) + sx * di as f32 + sy * dj as f32)).abs();
                }
            }
            if split || rough / 8.0 > 0.5 {
                row[i] = 1;
            }
        }
    });
    let mut tree = dsm.same_shape(0.0);
    tree.v.par_chunks_mut(nx).enumerate().for_each(|(j, row)| {
        if j == 0 || j == ny - 1 {
            return;
        }
        for i in 1..nx - 1 {
            let mut c = 0;
            for dj in [-1i64, 0, 1] {
                for di in [-1i64, 0, 1] {
                    c += raw[(j as i64 + dj) as usize * nx + (i as i64 + di) as usize] as u32;
                }
            }
            if raw[j * nx + i] == 1 && c >= 5 {
                row[i] = 1.0;
            }
        }
    });
    tree
}

/// Grey-scale morphological opening with a 3 x 3 window (min filter, then max filter).
/// Removes raised things narrower than ~3 m (overhead wires, poles, lone thin branches) that would
/// otherwise act as solid walls in the shadow test; roofs and tree crowns keep their height and shape.
fn open3(g: &mut Grid) {
    let (nx, ny) = (g.nx, g.ny);
    let pass = |src: &Vec<f32>, take_max: bool| -> Vec<f32> {
        let mut out = src.clone();
        out.par_chunks_mut(nx).enumerate().for_each(|(j, row)| {
            for i in 0..nx {
                let mut m = src[j * nx + i];
                for dj in -1i64..=1 {
                    for di in -1i64..=1 {
                        let (a, b) = (i as i64 + di, j as i64 + dj);
                        if a < 0 || b < 0 || a >= nx as i64 || b >= ny as i64 {
                            continue;
                        }
                        let v = src[b as usize * nx + a as usize];
                        m = if take_max { m.max(v) } else { m.min(v) };
                    }
                }
                row[i] = m;
            }
        });
        out
    };
    let eroded = pass(&g.v, false);
    g.v = pass(&eroded, true);
}
