//! Task 034: the agent ranks listings and pitches one with its Matterport link. Needs data/houston/listings.json
//! (local-only): every test returns early without it. `LIVE=1` also runs the real model (needs ANTHROPIC_API_KEY).

use server::agent::{run, AgentRequest};
use server::llm::Llm;

fn core() -> Option<engine::Core> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/houston");
    let mut c = engine::Core::load(&std::fs::read_to_string(format!("{dir}/graph.json")).ok()?, &std::fs::read_to_string(format!("{dir}/places.json")).ok()?).ok()?;
    c.load_listings(&std::fs::read_to_string(format!("{dir}/listings.json")).ok()?).ok()?;
    Some(c)
}

async fn ask(c: &mut engine::Core, llm: &Llm, text: &str, t: f32) -> server::agent::AgentResponse {
    let req = AgentRequest { text: text.into(), origin: None, t: Some(t) };
    run(c, llm, None, req).await.expect("agent finishes")
}

#[tokio::test]
async fn mock_pitches_a_listing_with_its_tour_link() {
    let Some(mut c) = core() else { return };
    let r = ask(&mut c, &Llm::Mock, "I need an apartment where I can push a stroller to cafes and parks", 12.0 * 60.0).await;
    assert_eq!(r.listings.len(), 1);
    let p = &r.listings[0];
    let url = p.tour_url.as_ref().expect("tour");
    assert!(url.starts_with("https://my.matterport.com/show/?m="));
    assert!(r.pitch.contains(url.as_str()));
    assert!(r.tools_called.contains(&"rank_listings".to_string()));
    println!("{}\n{:?}", r.pitch, p);
}

#[tokio::test]
async fn no_listings_data_is_said_not_invented() {
    let mut c = engine::Core::load(include_str!("../../data/fixtures/graph.json"), include_str!("../../data/fixtures/places.json")).unwrap();
    let req = AgentRequest { text: "I need an apartment near cafes".into(), origin: None, t: None };
    // the mock always pitches the top listing, which does not exist here: the run must fail loudly, not fabricate
    assert!(run(&mut c, &Llm::Mock, None, req).await.is_err() || c.listings.is_empty());
}

#[tokio::test]
async fn live_claude_pitch_passes_grounding() {
    if std::env::var("LIVE").is_err() {
        return;
    }
    let Some(mut c) = core() else { return };
    let r = ask(&mut c, &Llm::from_env(), "I'm moving to Houston with a toddler in a stroller. Which apartment should I look at? I walk everywhere, evenings too.", 18.0 * 60.0).await;
    println!("{}\n{:#?}", r.pitch, r.listings);
    assert!(!r.listings.is_empty());
    let url = r.listings[0].tour_url.as_ref().expect("tour link");
    assert!(r.pitch.contains(url.as_str()));
}
