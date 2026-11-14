//! Debug picture of the rasters: grey = ground (hillshaded), green = trees, orange = buildings,
//! brighter = taller. Only for eyeballing the data; nothing reads it.

use crate::geo::Grid;
use std::path::Path;

pub fn ndsm_png(dsm: &Grid, dtm: &Grid, tree: &Grid, path: &Path, step: usize) -> Result<(), String> {
    let (w, h) = (dsm.nx / step, dsm.ny / step);
    let mut px = vec![0u8; w * h * 3];
    for oy in 0..h {
        for ox in 0..w {
            let (i, j) = (ox * step, (h - 1 - oy) * step); // PNG rows go north -> south
            let k = j * dsm.nx + i;
            let ht = dsm.v[k] - dtm.v[k];
            // hillshade of the ground from the north-west
            let e = dtm.v[j * dsm.nx + (i + 1).min(dsm.nx - 1)] - dtm.v[k];
            let n = dtm.v[(j + 1).min(dsm.ny - 1) * dsm.nx + i] - dtm.v[k];
            let shade = (150.0 + (n - e) * 40.0).clamp(60.0, 230.0) as u8;
            let o = (oy * w + ox) * 3;
            let c = if ht < 2.0 {
                [shade, shade, shade]
            } else {
                let b = (80.0 + ht * 6.0).min(255.0) as u8;
                if tree.v[k] > 0.5 {
                    [b / 4, b, b / 4]
                } else {
                    [b, b / 2, 0]
                }
            };
            px[o..o + 3].copy_from_slice(&c);
        }
    }
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&px).map_err(|e| e.to_string())
}

/// Route picture for the demo: LiDAR background (dimmed), every street coloured by its shade in `slot`
/// (yellow = sun, dark blue = shade), and routes drawn on top in their colours. `m_per_px` metres per pixel.
pub fn routes_png(
    dsm: &Grid,
    dtm: &Grid,
    tree: &Grid,
    graph: &engine::graph::Graph,
    proj: &crate::geo::Proj,
    slot: usize,
    routes: &[(Vec<u32>, [u8; 3])],
    path: &Path,
    m_per_px: f64,
) -> Result<(), String> {
    // crop: all route nodes + 150 m
    let pts: Vec<(f64, f64)> = routes.iter().flat_map(|r| r.0.iter()).map(|&n| proj.xy(graph.nodes[n as usize].lon, graph.nodes[n as usize].lat)).collect();
    let pad = 150.0;
    let x0 = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - pad;
    let x1 = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) + pad;
    let y0 = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - pad;
    let y1 = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) + pad;
    let (w, h) = (((x1 - x0) / m_per_px) as usize, ((y1 - y0) / m_per_px) as usize);
    let mut px = vec![0u8; w * h * 3];
    let to_px = |x: f64, y: f64| ((x - x0) / m_per_px, (y1 - y) / m_per_px);
    for oy in 0..h {
        for ox in 0..w {
            let (x, y) = (x0 + (ox as f64 + 0.5) * m_per_px, y1 - (oy as f64 + 0.5) * m_per_px);
            let c = match (dsm.at(x, y), dtm.at(x, y), tree.at(x, y)) {
                (Some(s), Some(t), Some(tr)) if s - t > 2.5 => if tr > 0.5 { [40, 80, 40] } else { [110, 70, 40] },
                _ => [55, 55, 55],
            };
            px[(oy * w + ox) * 3..(oy * w + ox) * 3 + 3].copy_from_slice(&c);
        }
    }
    let mut plot = |x: f64, y: f64, c: [u8; 3], r: i64| {
        let (fx, fy) = to_px(x, y);
        for dy in -r..=r {
            for dx in -r..=r {
                let (ix, iy) = (fx as i64 + dx, fy as i64 + dy);
                if ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h {
                    let o = (iy as usize * w + ix as usize) * 3;
                    px[o..o + 3].copy_from_slice(&c);
                }
            }
        }
    };
    let mut line = |a: (f64, f64), b: (f64, f64), c: [u8; 3], r: i64| {
        let n = ((((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt() / (m_per_px * 0.5)).ceil() as usize).max(1);
        for k in 0..=n {
            let f = k as f64 / n as f64;
            plot(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, c, r);
        }
    };
    for e in &graph.edges {
        let s = e.shade.as_ref().map(|s| s[slot]).unwrap_or(0.0) as f64;
        // yellow (sun) -> dark blue (shade)
        let c = [(255.0 * (1.0 - s) + 30.0 * s) as u8, (220.0 * (1.0 - s) + 60.0 * s) as u8, (40.0 * (1.0 - s) + 160.0 * s) as u8];
        for g in e.geom.windows(2) {
            line(proj.xy(g[0][0], g[0][1]), proj.xy(g[1][0], g[1][1]), c, 1);
        }
    }
    for (nodes, c) in routes {
        for w2 in nodes.windows(2) {
            let (a, b) = (&graph.nodes[w2[0] as usize], &graph.nodes[w2[1] as usize]);
            // draw along the real edge shape when we can find it
            let e = graph.edges.iter().filter(|e| (e.from == w2[0] && e.to == w2[1]) || (e.from == w2[1] && e.to == w2[0])).min_by(|p, q| p.length_m.partial_cmp(&q.length_m).unwrap());
            match e {
                Some(e) => {
                    for g in e.geom.windows(2) {
                        line(proj.xy(g[0][0], g[0][1]), proj.xy(g[1][0], g[1][1]), *c, 2);
                    }
                }
                None => line(proj.xy(a.lon, a.lat), proj.xy(b.lon, b.lat), *c, 2),
            }
        }
    }
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&px).map_err(|e| e.to_string())
}
