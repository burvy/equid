//! prep-shade: USGS 3DEP LiDAR -> shade by time of day, tree cover, sunset viewpoints and stargazing spots.
//! See prep-shade/README.md for the method, step by step.
//!
//!   cargo run --release -p prep-shade            # all steps; fetch/raster skip if their output is cached
//!   cargo run --release -p prep-shade -- fetch   # 1. download LiDAR tiles  -> data/raw/lidar/ept/
//!   cargo run --release -p prep-shade -- raster  # 2. points -> 1 m DSM/DTM -> data/raw/lidar/*.bin
//!   cargo run --release -p prep-shade -- shade   # 3. sun + rasters -> graph.json shade/tree, places.json sunset + stargazing,
//!                                                #    stats + a 13:00 vs 16:00 route demo -> data/princeton/shade_stats.txt

mod demo;
mod edges;
mod ept;
mod geo;
mod image;
mod raster;
mod shade;
mod stargaze;
mod sun;
mod sunset;

use geo::{Grid, Proj};
use std::path::{Path, PathBuf};

/// Region box [west, south, east, north]; same as prep/src/main.rs (graph.json meta.bbox).
pub const BBOX: [f64; 4] = [-74.690, 40.330, -74.630, 40.370];
/// Extra LiDAR around the box so low-sun shadows and sunset skylines from outside are seen.
pub const PAD_M: f64 = 1000.0;
/// EPT depth inside the box (~4 pts/m^2) and in the padding ring (coarser is fine for skylines).
const CORE_DEPTH: u32 = 10;
const PAD_DEPTH: u32 = 9;
const CELL_M: f64 = 1.0;

pub fn padded_bbox() -> [f64; 4] {
    let dlat = PAD_M / 111_320.0;
    let dlon = PAD_M / (111_320.0 * ((BBOX[1] + BBOX[3]) / 2.0).to_radians().cos());
    [BBOX[0] - dlon, BBOX[1] - dlat, BBOX[2] + dlon, BBOX[3] + dlat]
}
pub fn proj() -> Proj {
    Proj::new((BBOX[0] + BBOX[2]) / 2.0, (BBOX[1] + BBOX[3]) / 2.0)
}

pub struct Paths {
    pub root: PathBuf,
    pub lidar: PathBuf,
    pub tiles: PathBuf,
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let lidar = root.join("data/raw/lidar");
    let paths = Paths { tiles: lidar.join("ept"), lidar, root };
    let step = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let t = std::time::Instant::now();
    let res = match step.as_str() {
        "fetch" => fetch(&paths, true),
        "raster" => raster(&paths, true),
        "shade" => edges::run(&paths),
        "all" => fetch(&paths, false).and_then(|_| raster(&paths, false)).and_then(|_| edges::run(&paths)),
        other => Err(format!("unknown step {other:?}; use fetch | raster | shade | all")),
    };
    if let Err(e) = res {
        eprintln!("prep-shade: {e}");
        std::process::exit(1);
    }
    eprintln!("prep-shade: {step} done in {:.1} s", t.elapsed().as_secs_f64());
}

fn fetch(p: &Paths, force: bool) -> Result<(), String> {
    if !force && p.tiles.join("tiles.txt").exists() {
        eprintln!("fetch: cached ({})", p.tiles.display());
        return Ok(());
    }
    ept::fetch(&p.tiles, BBOX, CORE_DEPTH, padded_bbox(), PAD_DEPTH)
}

fn raster(p: &Paths, force: bool) -> Result<(), String> {
    if !force && p.lidar.join("dsm.bin").exists() && p.lidar.join("dtm.bin").exists() && p.lidar.join("tree.bin").exists() && p.lidar.join("deck.bin").exists() {
        eprintln!("raster: cached ({})", p.lidar.display());
        return Ok(());
    }
    let (r, stats) = raster::build(&p.tiles, &proj(), padded_bbox(), CELL_M)?;
    eprint!("{stats}");
    r.dsm.save(&p.lidar.join("dsm.bin"))?;
    r.dtm.save(&p.lidar.join("dtm.bin"))?;
    r.deck.save(&p.lidar.join("deck.bin"))?;
    r.tree.save(&p.lidar.join("tree.bin"))?;
    std::fs::write(p.lidar.join("raster_stats.txt"), &stats).map_err(|e| e.to_string())?;
    // A picture of the height above ground (and a hillshade) to eyeball the data.
    image::ndsm_png(&r.dsm, &r.dtm, &r.tree, &p.lidar.join("ndsm.png"), 4)?;
    Ok(())
}

/// (dsm, dtm, deck, tree) from the raster step.
pub fn load_rasters(p: &Paths) -> Result<(Grid, Grid, Grid, Grid), String> {
    let l = |n: &str| Grid::load(&p.lidar.join(n));
    Ok((l("dsm.bin")?, l("dtm.bin")?, l("deck.bin")?, l("tree.bin")?))
}
