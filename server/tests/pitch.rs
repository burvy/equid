//! Pitch hygiene (no ids, no tool-call debris) and opening hours on the trip's day.

use server::agent::{clean_text, looks_like_osm_id};
use server::hours::{parse, today, weekday};

#[test]
fn weekday_of_demo_date() {
    assert_eq!(weekday("2026-11-15"), Some(6)); // Sunday
    assert_eq!(weekday("2026-11-14"), Some(5)); // Saturday
    assert_eq!(weekday("2024-02-29"), Some(3)); // Thursday (leap day)
    assert_eq!(weekday("2000-01-01"), Some(5)); // Saturday
    assert_eq!(weekday("garbage"), None);
}

#[test]
fn hours_common_forms() {
    // Conte's: Sunday 16:00-21:00
    let s = "Mo 11:30-21:00; Tu-Fr 11:30-22:00; Sa 16:00-22:00; Su 16:00-21:00";
    let v = today(s, 6, 1200.0).unwrap();
    assert_eq!(v["hours_today"], "16:00-21:00");
    assert_eq!(v["open_at_departure"], true);
    assert_eq!(v["closes_at"], "21:00");
    assert_eq!(today(s, 6, 1320.0).unwrap()["open_at_departure"], false);
    // a day list with a comma, and rules split by ", "
    assert_eq!(today("Mo-Th, Su 11:30-21:00; Fr, Sa 11:30-22:00", 6, 700.0).unwrap()["hours_today"], "11:30-21:00");
    assert_eq!(today("Su-Th 12:00-22:00, Fr-Sa 12:00-23:00", 4, 1360.0).unwrap()["hours_today"], "12:00-23:00");
    // split shifts
    let w = today("Mo-Tu 16:00-22:00; We-Th 11:30-14:30,16:00-22:00;Fr 11:30-14:30,16:00-23:00", 2, 900.0).unwrap();
    assert_eq!(w["hours_today"], "11:30-14:30, 16:00-22:00");
    assert_eq!(w["open_at_departure"], false);
    // off, past midnight, spill into the next morning
    assert_eq!(today("Mo-Th 10:30-21:00; Fr-Sa 10:30-21:30; Su off", 6, 700.0).unwrap()["hours_today"], "closed");
    let late = "Mo-Th 10:30-24:00; Fr-Sa 10:30-01:00; Su 10:30-24:00";
    assert_eq!(today(late, 4, 1430.0).unwrap()["closes_at"], "01:00");
    assert_eq!(today(late, 5, 30.0).unwrap()["open_at_departure"], true); // Friday's hours still running
    assert_eq!(today("Tu-Fr 10:00-15:00", 6, 700.0).unwrap()["hours_today"], "not listed");
    assert!(parse("24/7").is_some());
}

#[test]
fn hours_unreadable_is_none() {
    assert!(today("sunrise-sunset", 0, 600.0).is_none());
    assert!(today("Mo-Fr 09:00-17:00; PH off", 0, 600.0).is_none());
    assert!(today("Jan-Mar Mo 10:00-12:00", 0, 600.0).is_none());
}

#[test]
fn clean_text_strips_debris() {
    assert_eq!(clean_text("Small World Coffee is a close second.\"}"), "Small World Coffee is a close second.");
    assert_eq!(clean_text("\"Starbucks is close.\""), "Starbucks is close.");
    assert_eq!(clean_text("{\"pitch\": \"Go to Conte's.\"}"), "Go to Conte's.");
    assert_eq!(clean_text("Try the \"Dinky\" path."), "Try the \"Dinky\" path.");
    assert_eq!(clean_text("Plain text (with a note)."), "Plain text (with a note).");
}

#[test]
fn osm_ids_detected() {
    assert!(looks_like_osm_id("n10085089056"));
    assert!(looks_like_osm_id("w109782066"));
    assert!(looks_like_osm_id("r2918026"));
    assert!(!looks_like_osm_id("n12"));
    assert!(!looks_like_osm_id("north"));
    assert!(!looks_like_osm_id("w1x"));
}
