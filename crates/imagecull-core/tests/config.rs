//! The config file: what it may say, what it refuses to say, where a run finds
//! it, and how it merges with the flags (spec §11.1, ADR 0003).

use std::fs;
use std::path::{Path, PathBuf};

use imagecull_core::config::{self, ConfigFile, Env, Resolved};
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::rubric::{Builtin, RubricSource};

use tempfile::TempDir;

/// Every key, at a value that is not the default, so a key that silently
/// vanished would show up as a missing assertion.
const EVERY_KEY: &str = r#"
[server]
endpoint = "http://192.168.1.9:8000"
model = "clef-27b-oQ4e"
timeout = 300

[preview]
max_edge = 1024
jpeg_quality = 72

[scan]
recursive = true

[outputs]
json_scores = "answers.json"

[run]
rubric = "mine.toml"
shoot_context = "night street, pushed two stops"
dedupe = false
"#;

fn resolved(text: &str) -> Resolved {
    ConfigFile::from_toml(text)
        .expect("a valid config")
        .resolve(None)
}

fn error(text: &str) -> String {
    ConfigFile::from_toml(text)
        .expect_err("a config that must be rejected")
        .to_string()
}

fn env_in(dir: &Path) -> Env {
    Env {
        cwd: dir.to_path_buf(),
        xdg_config_home: None,
        home: None,
    }
}

#[test]
fn an_empty_file_sets_nothing() {
    assert_eq!(resolved(""), Resolved::default());
    assert_eq!(resolved("# nothing but a comment\n"), Resolved::default());
}

#[test]
fn a_file_may_set_only_the_section_it_cares_about() {
    let resolved = resolved("[server]\nendpoint = \"http://127.0.0.1:9000\"\n");
    assert_eq!(
        resolved.endpoint.as_deref(),
        Some("http://127.0.0.1:9000"),
        "the server section is enough on its own"
    );
    assert_eq!(resolved.max_edge, None, "and the rest stays unset");
    assert_eq!(resolved.rubric, None);
}

#[test]
fn every_key_reaches_the_resolved_values() {
    let resolved = resolved(EVERY_KEY);
    assert_eq!(
        resolved.endpoint.as_deref(),
        Some("http://192.168.1.9:8000")
    );
    assert_eq!(resolved.model.as_deref(), Some("clef-27b-oQ4e"));
    assert_eq!(resolved.timeout, Some(300));
    assert_eq!(resolved.max_edge, Some(1024));
    assert_eq!(resolved.jpeg_quality, Some(72));
    assert_eq!(resolved.recursive, Some(true));
    assert_eq!(resolved.json_scores, Some(PathBuf::from("answers.json")));
    assert_eq!(
        resolved.rubric,
        Some(RubricSource::File(PathBuf::from("mine.toml")))
    );
    assert_eq!(
        resolved.shoot_context.as_deref(),
        Some("night street, pushed two stops")
    );
    assert_eq!(resolved.dedupe, Some(false));
}

#[test]
fn an_unknown_key_is_fatal_and_names_itself() {
    let message = error("[preview]\nmax_edeg = 1568\n");
    assert!(
        message.contains("max_edeg"),
        "the message must name the key that has to change: {message}"
    );
}

#[test]
fn an_unknown_section_is_fatal_and_names_itself() {
    let message = error("[previews]\nmax_edge = 1568\n");
    assert!(
        message.contains("previews"),
        "the message must name the section that has to change: {message}"
    );
}

#[test]
fn the_deleted_rewrite_interval_is_an_unknown_key() {
    let message = error("[run]\ncsv_rewrite_every = 25\n");
    assert!(
        message.contains("csv_rewrite_every"),
        "a key that no longer does anything has to fail by name, not be ignored: {message}"
    );
}

#[test]
fn a_value_the_flags_would_reject_is_fatal() {
    let cases = [
        ("[preview]\nmax_edge = 0\n", "max_edge"),
        ("[preview]\njpeg_quality = 0\n", "jpeg_quality"),
        ("[preview]\njpeg_quality = 101\n", "jpeg_quality"),
        ("[server]\ntimeout = 0\n", "timeout"),
        ("[server]\nmodel = \"  \"\n", "model"),
        ("[server]\nendpoint = \"\"\n", "endpoint"),
    ];
    for (text, key) in cases {
        let message = error(text);
        assert!(
            message.contains(key),
            "rejecting {text:?} must name {key}: {message}"
        );
    }
}

#[test]
fn a_jpeg_quality_outside_one_to_one_hundred_states_the_range() {
    let message = error("[preview]\njpeg_quality = 101\n");
    assert!(
        message.contains("1") && message.contains("100"),
        "a range mistake must say what the range is: {message}"
    );
}

#[test]
fn a_file_that_is_not_toml_points_at_the_line() {
    let message = error("[preview\nmax_edge = 1568\n");
    assert!(
        message.contains("line") || message.contains("TOML"),
        "the parser's own location must survive: {message}"
    );
}

#[test]
fn a_relative_path_is_anchored_to_the_files_own_directory() {
    let base = Path::new("/Volumes/Shoot A");
    let file = ConfigFile::from_toml(EVERY_KEY).expect("a valid config");
    let resolved = file.resolve(Some(base));
    assert_eq!(
        resolved.rubric,
        Some(RubricSource::File(base.join("mine.toml")))
    );
    assert_eq!(resolved.json_scores, Some(base.join("answers.json")));
}

#[test]
fn an_absolute_path_is_left_alone() {
    let base = Path::new("/Volumes/Shoot A");
    let file = ConfigFile::from_toml(
        "[run]\nrubric = \"/etc/rubrics/mine.toml\"\n[outputs]\njson_scores = \"/tmp/answers.json\"\n",
    )
    .expect("a valid config");
    let resolved = file.resolve(Some(base));
    assert_eq!(
        resolved.rubric,
        Some(RubricSource::File(PathBuf::from("/etc/rubrics/mine.toml")))
    );
    assert_eq!(
        resolved.json_scores,
        Some(PathBuf::from("/tmp/answers.json"))
    );
}

#[test]
fn the_flags_win_over_the_file() {
    let file = resolved(EVERY_KEY);
    let flags = Resolved {
        max_edge: Some(1568),
        model: Some("clef-flash-oQ4e".to_string()),
        json_scores: Some(PathBuf::from("elsewhere.json")),
        ..Resolved::default()
    };
    let effective = file.override_with(flags).settle(None);
    assert_eq!(effective.params.max_edge, 1568, "--max-edge beat the file");
    assert_eq!(effective.model.as_deref(), Some("clef-flash-oQ4e"));
    assert_eq!(
        effective.outputs.json_scores,
        Some(PathBuf::from("elsewhere.json"))
    );
    assert_eq!(
        effective.params.jpeg_quality, 72,
        "a flag that said nothing leaves the file's value alone"
    );
}

#[test]
fn the_file_wins_over_the_built_in_default() {
    let effective = resolved(EVERY_KEY).settle(None);
    assert_eq!(effective.params.max_edge, 1024);
    assert_eq!(effective.params.jpeg_quality, 72);
    assert_eq!(effective.timeout, 300);
    assert_eq!(effective.endpoint, "http://192.168.1.9:8000");
    assert!(!effective.dedupe);
    assert!(effective.recursive);
    assert_eq!(
        effective.shoot_context.as_deref(),
        Some("night street, pushed two stops")
    );
}

#[test]
fn an_empty_source_settles_to_the_specs_defaults() {
    let effective = Resolved::default().settle(None);
    assert_eq!(effective.endpoint, "http://127.0.0.1:8000");
    assert_eq!(
        effective.model, None,
        "no alias means the rubric's own alias, which is part of its hash"
    );
    assert_eq!(effective.timeout, 120);
    assert_eq!(effective.params, Params::default());
    assert!(!effective.recursive);
    assert_eq!(effective.outputs.json_scores, None, "no report path at all");
    assert_eq!(effective.rubric, None);
    assert!(effective.dedupe);
    assert_eq!(effective.source, None);
    assert_eq!(
        effective.scan(None),
        ScanOptions {
            recursive: false,
            limit: None
        }
    );
}

#[test]
fn the_source_is_kept_for_provenance() {
    let source = PathBuf::from("/Volumes/Shoot A/imagecull.toml");
    let effective = Resolved::default().settle(Some(source.clone()));
    assert_eq!(effective.source, Some(source));
}

#[test]
fn a_rubric_name_selects_a_built_in_and_a_path_names_a_file() {
    assert_eq!(
        resolved("[run]\nrubric = \"holiday\"\n").rubric,
        Some(RubricSource::Builtin(Builtin::Holiday)),
        "a bare name is a built-in, exactly as on the command line"
    );
    assert_eq!(
        resolved("[run]\nrubric = \"rubrics/mine.toml\"\n").rubric,
        Some(RubricSource::File(PathBuf::from("rubrics/mine.toml")))
    );
}

#[test]
fn a_name_is_not_anchored_to_the_config_files_directory() {
    let base = Path::new("/Volumes/Shoot A");
    let file = ConfigFile::from_toml("[run]\nrubric = \"holiday\"\n").expect("a valid config");
    assert_eq!(
        file.resolve(Some(base)).rubric,
        Some(RubricSource::Builtin(Builtin::Holiday)),
        "there is no path to anchor: a name is a name wherever the file sits"
    );
}

#[test]
fn a_blank_rubric_is_fatal_and_lists_the_built_ins() {
    let message = error("[run]\nrubric = \"  \"\n");
    assert!(
        message.contains("rubric"),
        "the message must name the key: {message}"
    );
    for builtin in Builtin::ALL {
        assert!(
            message.contains(builtin.name()),
            "and say what could have been selected: {message}"
        );
    }
}

#[test]
fn the_shoot_config_is_found_in_the_working_directory() {
    let dir = TempDir::new().expect("a temp dir");
    fs::write(
        dir.path().join(config::SHOOT_CONFIG),
        "[preview]\nmax_edge = 1024\n",
    )
    .expect("write the shoot config");

    let loaded = config::discover(None, &env_in(dir.path()))
        .expect("discovery")
        .expect("the shoot config");

    assert_eq!(loaded.path, dir.path().join(config::SHOOT_CONFIG));
    assert_eq!(loaded.resolve().max_edge, Some(1024));
}

#[test]
fn an_explicit_config_wins_over_the_shoot_config() {
    let dir = TempDir::new().expect("a temp dir");
    fs::write(
        dir.path().join(config::SHOOT_CONFIG),
        "[preview]\nmax_edge = 1024\n",
    )
    .expect("write the shoot config");
    fs::write(dir.path().join("mine.toml"), "[preview]\nmax_edge = 800\n").expect("write");

    let loaded = config::discover(Some(Path::new("mine.toml")), &env_in(dir.path()))
        .expect("discovery")
        .expect("the explicit config");

    assert_eq!(loaded.path, dir.path().join("mine.toml"));
    assert_eq!(loaded.resolve().max_edge, Some(800));
}

#[test]
fn an_explicit_config_that_does_not_exist_is_fatal_not_a_fall_back() {
    let dir = TempDir::new().expect("a temp dir");
    fs::write(
        dir.path().join(config::SHOOT_CONFIG),
        "[preview]\nmax_edge = 1024\n",
    )
    .expect("write the shoot config");

    let message = config::discover(Some(Path::new("missing.toml")), &env_in(dir.path()))
        .expect_err("a missing --config target must not fall back")
        .to_string();

    assert!(
        message.contains("missing.toml") && message.contains("does not exist"),
        "the message must say which file is missing: {message}"
    );
}

#[test]
fn the_shoot_config_wins_over_the_user_config() {
    let dir = TempDir::new().expect("a temp dir");
    let user = TempDir::new().expect("a temp home");
    fs::write(
        dir.path().join(config::SHOOT_CONFIG),
        "[preview]\nmax_edge = 1024\n",
    )
    .expect("write the shoot config");
    write_user_config(user.path(), "[preview]\nmax_edge = 800\n");

    let env = Env {
        cwd: dir.path().to_path_buf(),
        xdg_config_home: Some(user.path().to_path_buf()),
        home: None,
    };
    let loaded = config::discover(None, &env)
        .expect("discovery")
        .expect("a config");

    assert_eq!(
        loaded.resolve().max_edge,
        Some(1024),
        "one file wins whole; they never layer"
    );
}

#[test]
fn the_user_config_is_found_when_no_shoot_config_exists() {
    let dir = TempDir::new().expect("a temp dir");
    let user = TempDir::new().expect("a temp config home");
    write_user_config(user.path(), "[preview]\nmax_edge = 800\n");

    let env = Env {
        cwd: dir.path().to_path_buf(),
        xdg_config_home: Some(user.path().to_path_buf()),
        home: None,
    };
    let loaded = config::discover(None, &env)
        .expect("discovery")
        .expect("a config");

    assert_eq!(
        loaded.path,
        user.path()
            .join(config::USER_CONFIG_DIR)
            .join(config::USER_CONFIG_FILE)
    );
    assert_eq!(loaded.resolve().max_edge, Some(800));
}

#[test]
fn the_user_config_falls_back_to_dot_config_under_home() {
    let dir = TempDir::new().expect("a temp dir");
    let home = TempDir::new().expect("a temp home");
    write_user_config(&home.path().join(".config"), "[preview]\nmax_edge = 640\n");

    let env = Env {
        cwd: dir.path().to_path_buf(),
        xdg_config_home: None,
        home: Some(home.path().to_path_buf()),
    };
    let loaded = config::discover(None, &env)
        .expect("discovery")
        .expect("a config");

    assert_eq!(loaded.resolve().max_edge, Some(640));
}

#[test]
fn nothing_found_is_not_an_error() {
    let dir = TempDir::new().expect("a temp dir");
    let env = env_in(dir.path());
    assert_eq!(
        config::discover(None, &env).expect("discovery"),
        None,
        "no config file anywhere is a run with the built-in defaults"
    );
}

#[test]
fn a_malformed_discovered_file_names_its_path() {
    let dir = TempDir::new().expect("a temp dir");
    let path = dir.path().join(config::SHOOT_CONFIG);
    fs::write(&path, "[preview]\nmax_edeg = 1024\n").expect("write");

    let message = config::discover(None, &env_in(dir.path()))
        .expect_err("a typo must stop the run")
        .to_string();

    assert!(
        message.contains(&path.display().to_string()) && message.contains("max_edeg"),
        "the message must name the file and the key: {message}"
    );
}

#[test]
fn the_committed_example_parses() {
    // The example is the reference a photographer copies, so it must never rot
    // into a file imagecull itself would reject.
    ConfigFile::from_toml(include_str!("../../../imagecull.toml.example"))
        .expect("imagecull.toml.example must be a valid config");
}

/// Write a user-level config under `base`, the way discovery expects it.
fn write_user_config(base: &Path, text: &str) {
    let dir = base.join(config::USER_CONFIG_DIR);
    fs::create_dir_all(&dir).expect("create the config directory");
    fs::write(dir.join(config::USER_CONFIG_FILE), text).expect("write the user config");
}

#[test]
fn the_report_keys_are_refused_with_an_explanation_rather_than_a_typo_message() {
    let message = error("[outputs]\ncsv = \"scores.csv\"\n");
    assert!(
        message.contains("sidecar"),
        "the refusal has to say where ratings go now: {message}"
    );
    assert!(message.contains("outputs.csv"), "{message}");

    let message = error("[outputs]\nabsolute = true\n");
    assert!(message.contains("outputs.absolute"), "{message}");
}
