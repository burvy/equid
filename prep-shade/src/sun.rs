//! Where the sun is: NOAA's "General Solar Position Calculations" (fractional-year series,
//! accurate to a few arc-minutes), the same formulas prep uses for meta.sunrise_min/sunset_min.

use std::f64::consts::PI;

pub struct Day {
    year: i32,
    pub doy: u32,
    leap: bool,
    pub utc_offset_h: f64,
}

impl Day {
    /// `date` = "YYYY-MM-DD", local time zone America/New_York (EST/EDT by the US rule).
    pub fn parse(date: &str) -> Result<Day, String> {
        let p: Vec<i64> = date.split('-').filter_map(|s| s.parse().ok()).collect();
        if p.len() != 3 {
            return Err(format!("bad date {date:?}"));
        }
        let (year, month, day) = (p[0] as i32, p[1] as u32, p[2] as u32);
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let doy = day_of_year(year, month, day);
        // US DST: from the 2nd Sunday of March to the 1st Sunday of November.
        let dst_start = day_of_year(year, 3, 8 + (7 - weekday(year, 3, 8)) % 7);
        let dst_end = day_of_year(year, 11, 1 + (7 - weekday(year, 11, 1)) % 7);
        let utc_offset_h = if doy >= dst_start && doy < dst_end { -4.0 } else { -5.0 };
        Ok(Day { year, doy, leap, utc_offset_h })
    }

    /// Sun direction at local time `t_min` (minutes since midnight) seen from (lat, lon):
    /// (altitude above the horizon in degrees, including refraction; azimuth clockwise from north in degrees).
    pub fn sun(&self, t_min: f64, lat: f64, lon: f64) -> (f64, f64) {
        let _ = self.year;
        let hour = t_min / 60.0 - self.utc_offset_h; // UTC hours
        let g = 2.0 * PI / if self.leap { 366.0 } else { 365.0 } * (self.doy as f64 - 1.0 + (hour - 12.0) / 24.0);
        let eqtime = 229.18
            * (0.000075 + 0.001868 * g.cos() - 0.032077 * g.sin() - 0.014615 * (2.0 * g).cos() - 0.040849 * (2.0 * g).sin());
        let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos()
            + 0.000907 * (2.0 * g).sin()
            - 0.002697 * (3.0 * g).cos()
            + 0.00148 * (3.0 * g).sin();
        let tst = t_min + eqtime + 4.0 * lon - 60.0 * self.utc_offset_h; // true solar time, minutes
        let ha = (tst / 4.0 - 180.0).to_radians(); // hour angle: 0 at solar noon
        let phi = lat.to_radians();
        // Unit vector to the sun in east/north/up.
        let e = -decl.cos() * ha.sin();
        let n = phi.cos() * decl.sin() - phi.sin() * decl.cos() * ha.cos();
        let u = phi.sin() * decl.sin() + phi.cos() * decl.cos() * ha.cos();
        let alt = u.asin().to_degrees();
        let az = e.atan2(n).to_degrees().rem_euclid(360.0);
        (alt + refraction_deg(alt), az)
    }
}

/// Atmospheric refraction lifts the sun near the horizon (~0.5 deg at 0); Bennett's formula.
fn refraction_deg(alt: f64) -> f64 {
    if alt < -1.0 {
        return 0.0;
    }
    1.02 / ((alt + 10.3 / (alt + 5.11)).to_radians().tan()) / 60.0
}

fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let cum = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    cum[(month - 1) as usize] + day + u32::from(leap && month > 2)
}

/// 0 = Sunday (Sakamoto's method).
fn weekday(year: i32, month: u32, day: u32) -> u32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    ((y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day as i32) % 7) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    const LAT: f64 = 40.35;
    const LON: f64 = -74.66;

    #[test]
    fn time_zone() {
        assert_eq!(Day::parse("2026-11-15").unwrap().utc_offset_h, -5.0);
        assert_eq!(Day::parse("2026-07-01").unwrap().utc_offset_h, -4.0);
        assert_eq!(weekday(2026, 11, 14), 6); // Saturday
    }

    #[test]
    fn princeton_mid_november() {
        // NOAA solar calculator, Princeton 2026-11-15: solar noon ~11:43 EST, noon altitude ~31.5 deg,
        // sunset ~16:42 at azimuth ~245-246 deg (cos az = sin decl / cos lat, decl -18.6).
        let d = Day::parse("2026-11-15").unwrap();
        let (best_t, best_alt) = (600..800).map(|t| (t, d.sun(t as f64, LAT, LON).0)).fold((0, -90.0), |a, b| if b.1 > a.1 { b } else { a });
        assert!((best_t as i32 - 703).abs() <= 3, "solar noon {best_t}");
        assert!((best_alt - 31.5).abs() < 0.7, "noon altitude {best_alt}");
        let (_, az_noon) = d.sun(best_t as f64, LAT, LON);
        assert!((az_noon - 180.0).abs() < 1.5, "noon azimuth {az_noon}");
        // apparent sunset: refracted centre at -0.27 deg (solar radius) ~ 16:42
        let set = (900..1100).find(|&t| d.sun(t as f64, LAT, LON).0 < -0.27).unwrap();
        assert!((set as i32 - 1002).abs() <= 4, "sunset {set}");
        let (_, az_set) = d.sun(set as f64, LAT, LON);
        assert!((az_set - 245.5).abs() < 1.5, "sunset azimuth {az_set}");
    }
}
