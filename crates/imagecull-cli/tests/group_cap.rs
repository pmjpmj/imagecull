//! `[group] max_images` as a photographer sets it: a shoot config, a folder of
//! one moment, and the requests a real socket receives.
//!
//! The cap is scheduling, so the proof is a request count rather than a verdict:
//! four frames at a cap of two cost three comparisons (two pairs, then the two
//! winners), where the same folder without the key costs one. The fixtures are
//! the core's own — a NEF-shaped file and a loopback stub — because a second copy
//! of either would drift from the one the model-free tests already trust.

#[path = "../../imagecull-core/tests/support/mod.rs"]
mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::json;
use support::stub::{Received, Stub};

/// The alias a group's comparison asks, and the one the stub serves.
const ALIAS: &str = "clef-flash-oQ4e";

/// A folder of `names`, all one moment: the same second, a few hundredths
/// apart, each a little brighter than the last so no two frames are identical.
fn one_moment(names: &[&'static str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    let scene = support::scenes::scene(640, 427);
    for (index, name) in names.iter().enumerate() {
        let step = f32::from(u16::try_from(index).expect("a short group"));
        let image = support::scenes::brighter(&scene, step * 0.05);
        let facts = support::ExifFacts {
            captured: Some("2026:11:02 10:00:00".to_string()),
            subsecond: Some(format!("{:02}", 10 + index)),
            serial: Some("3001".to_string()),
            iso: Some(200),
            exposure_time: Some((1, 500)),
            f_number: Some((28, 10)),
            focal_length: Some((50, 1)),
        };
        fs::write(
            dir.path().join(name),
            support::nef_with(1, &[support::jpeg_of(&image, 92)], &facts),
        )
        .expect("write a NEF");
    }
    dir
}

/// The comparative answer document the stub sends: the first frame of every
/// batch, so the run is decided and deterministic.
fn keeps_the_first() -> String {
    json!({
        "model": ALIAS,
        "answers": {
            "keeper": {"type": "choice", "choice": "1", "confidence": 0.9}
        },
        "usage": {"input_tokens": 4154, "output_tokens": 0},
    })
    .to_string()
}

/// Point the shoot folder at `stub`, adding `group` as its `[group]` section.
fn shoot_config(shoot: &Path, stub: &Stub, group: &str) {
    fs::write(
        shoot.join("imagecull.toml"),
        format!(
            "[server]\nendpoint = \"{}/v1/systemone\"\nmodel = \"{ALIAS}\"\n\n{group}",
            stub.url()
        ),
    )
    .expect("a shoot config");
}

/// Run `groups` over `shoot`, with the shoot config discovered in the working
/// directory — which is how a photographer runs it, and what proves the key
/// reached the cull rather than only the parser.
fn groups(shoot: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_imagecull"))
        .arg("groups")
        .arg(shoot)
        .arg("--out")
        .arg(shoot.join("groups.csv"))
        .current_dir(shoot)
        .output()
        .expect("the binary runs")
}

/// The decision requests a stub received: its preflight GETs are not them.
fn comparisons(stub: &Stub) -> Vec<Received> {
    stub.requests()
        .into_iter()
        .filter(|request| request.path == "/v1/systemone")
        .collect()
}

/// How many images one request carried.
fn images(request: &Received) -> usize {
    request.body["images"]
        .as_array()
        .map(std::vec::Vec::len)
        .unwrap_or_default()
}

#[test]
fn a_shoot_that_caps_the_comparison_pays_one_request_per_pair() {
    let shoot = one_moment(&[
        "DSC_0001.NEF",
        "DSC_0002.NEF",
        "DSC_0003.NEF",
        "DSC_0004.NEF",
    ]);
    let stub = Stub::serving(
        vec![
            (200, keeps_the_first()),
            (200, keeps_the_first()),
            (200, keeps_the_first()),
        ],
        &[ALIAS],
    );
    shoot_config(shoot.path(), &stub, "[group]\nmax_images = 2\n");

    let output = groups(shoot.path());

    assert_eq!(
        output.status.code(),
        Some(0),
        "the group is decided: {output:?}"
    );
    let compared = comparisons(&stub);
    assert_eq!(
        compared.len(),
        3,
        "four frames at a cap of two is two pairs and then the pair of winners, not the one \
         request a group of four would otherwise cost: {} {compared:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        compared.iter().all(|request| images(request) == 2),
        "no request carries more than the cap: {:?}",
        compared.iter().map(images).collect::<Vec<_>>()
    );
}

#[test]
fn a_shoot_that_says_nothing_compares_the_group_in_one_request() {
    // The same folder without the key, so the two tests read as one claim: the
    // key is what splits the group, and its absence keeps the measured batch.
    let shoot = one_moment(&[
        "DSC_0001.NEF",
        "DSC_0002.NEF",
        "DSC_0003.NEF",
        "DSC_0004.NEF",
    ]);
    let stub = Stub::serving(vec![(200, keeps_the_first())], &[ALIAS]);
    shoot_config(shoot.path(), &stub, "");

    let output = groups(shoot.path());

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let compared = comparisons(&stub);
    assert_eq!(
        compared.len(),
        1,
        "an absent cap is the measured twenty, so the whole group is one request: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(images(&compared[0]), 4);
}
