//! Day of the week and a small OSM `opening_hours` reader, so the agent can say "open until 21:00 tonight"
//! instead of hedging. Covers the common forms (`Mo-Fr 11:30-22:00; Sa 16:00-22:00`, `Mo-Th, Su ...`,
//! `Su off`, several ranges per day, past-midnight ends like `10:30-01:00`). Anything else
//! (`sunrise-sunset`, `PH off`, month rules...) gives `None`: the agent then sees the raw string only.

pub const DAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
pub const DAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// Weekday of a `YYYY-MM-DD` date, Monday = 0 (Sakamoto's method).
pub fn weekday(date: &str) -> Option<usize> {
    let mut it = date.trim().splitn(3, '-').map(|x| x.parse::<i64>().ok());
    let (mut y, m, d) = (it.next()??, it.next()??, it.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    if m < 3 {
        y -= 1;
    }
    let sun0 = (y + y / 4 - y / 100 + y / 400 + T[m as usize - 1] + d).rem_euclid(7); // Sunday = 0
    Some(((sun0 + 6) % 7) as usize)
}

#[derive(Debug, PartialEq)]
enum Tok {
    Day(usize),
    Dash,
    Sep,
    Time(u32, u32),
    Off,
}

fn hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 24 && m < 60 && h * 60 + m <= 1440).then_some(h * 60 + m)
}

fn tokens(s: &str) -> Option<Vec<Tok>> {
    let mut out = vec![];
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b',' || c == b';' {
            out.push(Tok::Sep);
            i += 1;
        } else if c == b'-' {
            out.push(Tok::Dash);
            i += 1;
        } else if c.is_ascii_digit() {
            // HH:MM-HH:MM as one token
            let st = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b':' || b[i] == b'-') {
                i += 1;
            }
            let (a, z) = s[st..i].split_once('-')?;
            out.push(Tok::Time(hhmm(a)?, hhmm(z)?));
        } else if s[i..].starts_with("off") || s[i..].starts_with("closed") {
            out.push(Tok::Off);
            i += if s[i..].starts_with("off") { 3 } else { 6 };
        } else {
            let d = DAYS.iter().position(|d| s[i..].starts_with(d))?;
            out.push(Tok::Day(d));
            i += 2;
        }
    }
    Some(out)
}

/// Opening ranges (minutes since midnight; `end` may exceed 1440 for past midnight) for each weekday.
/// `Some(vec![])` = closed that day; `None` for a day = not mentioned.
pub fn parse(s: &str) -> Option<[Option<Vec<(u32, u32)>>; 7]> {
    let s = s.trim();
    if s == "24/7" {
        return Some(std::array::from_fn(|_| Some(vec![(0, 1440)])));
    }
    let toks = tokens(s)?;
    let mut week: [Option<Vec<(u32, u32)>>; 7] = Default::default();
    let mut i = 0;
    while i < toks.len() {
        // day selector: Day ( - Day )? ( , Day ( - Day )? )*
        let mut days = [false; 7];
        let mut any = false;
        while let Some(Tok::Day(a)) = toks.get(i) {
            let a = *a;
            let b = if toks.get(i + 1) == Some(&Tok::Dash) {
                match toks.get(i + 2) {
                    Some(Tok::Day(b)) => {
                        i += 2;
                        *b
                    }
                    _ => return None,
                }
            } else {
                a
            };
            let mut d = a;
            loop {
                days[d] = true;
                if d == b {
                    break;
                }
                d = (d + 1) % 7; // wraps: Su-Th, Fr-Mo
            }
            any = true;
            i += 1;
            if toks.get(i) == Some(&Tok::Sep) && matches!(toks.get(i + 1), Some(Tok::Day(_))) {
                i += 1;
            } else {
                break;
            }
        }
        if !any {
            days = [true; 7]; // "10:00-18:00" alone = every day
        }
        // times: Off | Time ( , Time )*
        let mut ranges = vec![];
        match toks.get(i) {
            Some(Tok::Off) => i += 1,
            Some(Tok::Time(..)) => {
                while let Some(Tok::Time(a, z)) = toks.get(i) {
                    ranges.push((*a, if z <= a { z + 1440 } else { *z }));
                    i += 1;
                    if toks.get(i) == Some(&Tok::Sep) && matches!(toks.get(i + 1), Some(Tok::Time(..))) {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => return None,
        }
        for d in 0..7 {
            if days[d] {
                week[d] = Some(ranges.clone()); // later rules override earlier ones (OSM semantics)
            }
        }
        match toks.get(i) {
            None => {}
            Some(Tok::Sep) => i += 1,
            _ => return None,
        }
    }
    Some(week)
}

fn fmt(m: u32) -> String {
    let m = m % 1440;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// What the agent sees about one place's hours on `day` at minute `t`: e.g.
/// `{"hours_today": "16:00-21:00", "open_at_departure": true}`, or `None` if the string can't be read.
pub fn today(s: &str, day: usize, t: f32) -> Option<serde_json::Value> {
    let week = parse(s)?;
    let t = t as u32;
    // yesterday's past-midnight range still running (e.g. Fr 10:30-01:00 at Sa 00:30)
    let spill = week[(day + 6) % 7].as_ref().is_some_and(|r| r.iter().any(|&(_, z)| z > 1440 && t < z - 1440));
    let Some(r) = &week[day] else {
        return Some(serde_json::json!({"hours_today": "not listed", "open_at_departure": spill}));
    };
    let open = spill || r.iter().any(|&(a, z)| a <= t && t < z);
    let hours = if r.is_empty() { "closed".to_string() } else { r.iter().map(|&(a, z)| format!("{}-{}", fmt(a), fmt(z))).collect::<Vec<_>>().join(", ") };
    let mut v = serde_json::json!({"hours_today": hours, "open_at_departure": open});
    if open {
        if let Some(&(_, z)) = r.iter().find(|&&(a, z)| a <= t && t < z) {
            v["closes_at"] = serde_json::json!(fmt(z));
        }
    }
    Some(v)
}
