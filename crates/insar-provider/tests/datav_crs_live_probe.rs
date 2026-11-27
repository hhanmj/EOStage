//! Live check for whether DataV Hong Kong / Macao (and Taiwan) geometry is
//! GCJ-02 or already WGS84. Ignored by default: it reads the public DataV and
//! Nominatim endpoints and must only run on an isolated desktop with network
//! approval. Run:
//! `cargo test -p insar-provider --test datav_crs_live_probe -- --ignored --nocapture`

use std::thread;
use std::time::Duration;

use insar_provider::boundary::{
    classify_datav_crs, DatavCrsVerdict, DATAV_BOUNDARY_ROOT, EOSTAGE_USER_AGENT,
};
use serde_json::Value;

fn get_json(client: &reqwest::blocking::Client, url: &str) -> Value {
    let response = client.get(url).send().expect("bounded public request");
    assert!(
        response.status().is_success(),
        "{url}: {}",
        response.status()
    );
    response.json().expect("JSON response")
}

#[test]
#[ignore = "live public DataV and Nominatim requests"]
fn datav_hong_kong_macao_and_taiwan_crs_against_osm_outlines() {
    let client = reqwest::blocking::Client::builder()
        .user_agent(EOSTAGE_USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("probe client");
    // (DataV adcode, OSM relation, expected OSM name fragment)
    let cases = [
        ("810000", "R913110", "香港"),
        ("820000", "R1867188", "澳門"),
        ("710000", "R449220", "臺灣"),
    ];
    let mut verdicts = Vec::new();
    for (adcode, relation, name) in cases {
        let datav = get_json(&client, &format!("{DATAV_BOUNDARY_ROOT}/{adcode}.json"));
        thread::sleep(Duration::from_millis(1_100));
        let osm = get_json(
            &client,
            &format!(
                "https://nominatim.osm.org/lookup?osm_ids={relation}&format=geojson&polygon_geojson=1&accept-language=zh-TW"
            ),
        );
        let feature = &osm["features"][0];
        let osm_name = feature["properties"]["name"].as_str().unwrap_or_default();
        assert!(
            osm_name.contains(name),
            "{relation} resolved to {osm_name:?}, not {name}"
        );
        let verdict = classify_datav_crs(&datav, &feature["geometry"]);
        println!("DataV {adcode} vs OSM {relation} ({osm_name}): {verdict:?}");
        verdicts.push((adcode, verdict));
    }
    assert!(
        verdicts
            .iter()
            .all(|(_, verdict)| *verdict != DatavCrsVerdict::Inconclusive),
        "{verdicts:?}"
    );
}
