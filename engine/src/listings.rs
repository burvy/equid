//! Apartment / home listings (optional, local-only `listings.json`, written by prep from the CoStar kit) and
//! how well each one fits a metric.
//!
//! Score of a listing under a metric (documented here, quoted by the agent):
//! run the cost field from the listing's front-door node (arcs are directed: costs are leaving home).
//! For each category in the *lens* (default: everyday destinations), take the cheapest place of that category;
//! it earns `1 - minutes / budget` (linear, 0 at or beyond the budget, and 0 if unreachable). The score is the
//! mean over the lens times 100. So 100 = every kind of place is at the door, 0 = none within the budget, and a
//! metric that makes streets costly (night, stroller, flood) lowers listings whose destinations sit behind them.

use crate::{metric::Metric, Core};
use serde::{Deserialize, Serialize};

/// Everyday destinations scored by default.
pub const DEFAULT_LENS: &[&str] = &["restaurant", "cafe", "park", "pharmacy", "playground", "library", "museum", "garden", "ice_cream"];

/// A listing as prep writes it (only the fields the engine and agent use; unknown fields are ignored).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Listing {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    pub node: u32,
    #[serde(default)]
    pub rent_min: Option<f64>,
    #[serde(default)]
    pub rent_max: Option<f64>,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub beds_min: Option<f64>,
    #[serde(default)]
    pub beds_max: Option<f64>,
    #[serde(default)]
    pub baths_min: Option<f64>,
    #[serde(default)]
    pub baths_max: Option<f64>,
    #[serde(default)]
    pub sqft_min: Option<f64>,
    #[serde(default)]
    pub sqft_max: Option<f64>,
    #[serde(default)]
    pub year_built: Option<f64>,
    #[serde(default)]
    pub amenities: Vec<String>,
    #[serde(default)]
    pub matterport: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ListingsFile {
    listings: Vec<Listing>,
}

impl Listing {
    /// Available to rent or buy right now (CoStar status "active").
    pub fn available(&self) -> bool {
        self.status == "active"
    }

    /// First Matterport tour of the listing, as a link.
    pub fn tour_url(&self) -> Option<String> {
        self.matterport.first().map(|m| format!("https://my.matterport.com/show/?m={m}"))
    }
}

/// One category's nearest place from a listing.
#[derive(Debug, Clone, Serialize)]
pub struct Nearest {
    pub category: String,
    pub place: Option<String>,
    pub minutes: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ranked {
    /// Index into `Core::listings`.
    pub index: usize,
    /// 0..100, see the module docs.
    pub score: f32,
    /// Places (of any category) reachable within the budget.
    pub reachable_places: usize,
    /// Per lens category, cheapest first (unreachable ones last, minutes = INFINITY).
    pub nearest: Vec<Nearest>,
}

impl Core {
    /// Load a `listings.json`; returns the number of listings. Errors if a listing's node is not in the graph.
    pub fn load_listings(&mut self, json: &str) -> Result<usize, String> {
        let f: ListingsFile = serde_json::from_str(json).map_err(|e| format!("listings.json: {e}"))?;
        if let Some(l) = f.listings.iter().find(|l| l.node as usize >= self.graph.nodes.len()) {
            return Err(format!("listing {} has node {} outside the graph", l.id, l.node));
        }
        self.listings = f.listings;
        Ok(self.listings.len())
    }

    /// Rank listings (descending score; ties by id) under `metric` at `t_min`, over the lens categories
    /// (empty = `DEFAULT_LENS`), with a cost budget in minutes. `only_available` keeps status "active" only.
    pub fn rank_listings(&self, metric: &Metric, t_min: f32, budget_min: f32, lens: &[String], only_available: bool) -> Result<Vec<Ranked>, String> {
        if !(budget_min > 0.0 && budget_min.is_finite()) {
            return Err("budget must be a positive number of minutes".into());
        }
        let lens: Vec<String> = if lens.is_empty() { DEFAULT_LENS.iter().map(|s| s.to_string()).collect() } else { lens.to_vec() };
        let budget_s = budget_min * 60.0;
        let mut out = vec![];
        for (index, l) in self.listings.iter().enumerate() {
            if only_available && !l.available() {
                continue;
            }
            let f = self.field(l.node, metric, t_min)?;
            let mut nearest: Vec<Nearest> = lens
                .iter()
                .map(|c| {
                    let best = self
                        .places
                        .places
                        .iter()
                        .zip(&f.place_cost)
                        .filter(|(p, _)| &p.category == c)
                        .min_by(|a, b| a.1.total_cmp(b.1));
                    Nearest {
                        category: c.clone(),
                        place: best.and_then(|(p, _)| p.name.clone()),
                        minutes: best.map(|(_, s)| *s / 60.0).filter(|m| m.is_finite()).unwrap_or(f32::INFINITY),
                    }
                })
                .collect();
            let score = nearest.iter().map(|n| (1.0 - n.minutes / budget_min).max(0.0)).sum::<f32>() / nearest.len() as f32 * 100.0;
            nearest.sort_by(|a, b| a.minutes.total_cmp(&b.minutes));
            let reachable_places = f.place_cost.iter().filter(|c| **c <= budget_s).count();
            out.push(Ranked { index, score, reachable_places, nearest });
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| self.listings[a.index].id.cmp(&self.listings[b.index].id)));
        Ok(out)
    }
}
