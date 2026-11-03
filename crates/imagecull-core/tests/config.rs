//! The config file: what it may say, what it refuses to say, where a run finds
//! it, and how it merges with the flags (spec §11.1, ADR 0003).

use std::fs;
use std::path::{Path, PathBuf};

use imagecull_core::config::{self, ConfigFile, Env, Resolved};
use imagecull_core::group::Thresholds;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::keeper;
use imagecull_core::preprocess::Params;
use imagecull_core::rubric::{Builtin, RubricSource};
use imagecull_core::secret::Secret;

use tempfile::TempDir;

/// Every key, at a value that is not the default, so a key that silently
/// vanished would show up as a missing assertion.
const EVERY_KEY: &str = r#"
[server]
endpoint = "http://192.168.1.9:8000/v1/systemone/{model}"
model = "clef-27b-oQ4e"
token_env = "CLOUDFLARE_API_TOKEN"
token = "a-literal-secret"
truncate = false
timeout = 300

[scan]
recursive = true

[score]
max_edge = 1024
jpeg_quality = 72
json_scores = "answers.json"
rubric = "mine.toml"
shoot_context = "night street, pushed two stops"
dedupe = false

[group]
gap = 0.75
similarity = 0.96
exposure_stops = 1.5
max_edge = 700
jpeg_quality = 60
cull_jobs = 2
max_images = 4
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
    assert_eq!(resolved.score_max_edge, None, "and the rest stays unset");
    assert_eq!(resolved.rubric, None);
}

#[test]
fn every_key_reaches_the_resolved_values() {
    let resolved = resolved(EVERY_KEY);
    assert_eq!(
        resolved.endpoint.as_deref(),
        Some("http://192.168.1.9:8000/v1/systemone/{model}"),
        "the address is carried as written, not interpolated here"
    );
    assert_eq!(resolved.model.as_deref(), Some("clef-27b-oQ4e"));
    assert_eq!(
        resolved.token_env.as_deref(),
        Some("CLOUDFLARE_API_TOKEN"),
        "the variable's name is plumbing: nothing resolves it at settle time"
    );
    assert_eq!(
        resolved.token.as_ref().map(Secret::as_str),
        Some("a-literal-secret"),
        "the literal is carried as a secret, which is how it stays unprintable"
    );
    assert_eq!(resolved.truncate, Some(false));
    assert_eq!(resolved.timeout, Some(300));
    assert_eq!(resolved.score_max_edge, Some(1024));
    assert_eq!(resolved.score_jpeg_quality, Some(72));
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
    assert_eq!(resolved.group_gap, Some(0.75));
    assert_eq!(resolved.group_similarity, Some(0.96));
    assert_eq!(resolved.group_exposure_stops, Some(1.5));
    assert_eq!(
        resolved.group_max_edge,
        Some(700),
        "a comparison's edge is its own key, not [score]'s"
    );
    assert_eq!(resolved.group_jpeg_quality, Some(60));
    assert_eq!(
        resolved.group_cull_jobs,
        Some(2),
        "a shoot may carry the degree its comparison runs at"
    );
    assert_eq!(
        resolved.group_max_images,
        Some(4),
        "and the most frames one comparison may carry"
    );
}

#[test]
fn an_unknown_key_is_fatal_and_names_itself() {
    let message = error("[score]\nmax_edeg = 1568\n");
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
fn the_retired_sections_name_the_key_that_replaced_them() {
    for (text, old, new) in [
        (
            "[preview]\nmax_edge = 1024\n",
            "preview.max_edge",
            "score.max_edge",
        ),
        (
            "[preview]\njpeg_quality = 72\n",
            "preview.jpeg_quality",
            "score.jpeg_quality",
        ),
        (
            "[outputs]\njson_scores = \"answers.json\"\n",
            "outputs.json_scores",
            "score.json_scores",
        ),
        (
            "[run]\nrubric = \"holiday\"\n",
            "run.rubric",
            "score.rubric",
        ),
        (
            "[run]\nshoot_context = \"dusk\"\n",
            "run.shoot_context",
            "score.shoot_context",
        ),
        ("[run]\ndedupe = false\n", "run.dedupe", "score.dedupe"),
    ] {
        let message = error(text);
        assert!(
            message.contains(old) && message.contains(new),
            "rejecting {text:?} must name {old} and where it went ({new}): {message}"
        );
    }
}

#[test]
fn a_retired_key_is_refused_before_its_value_is_judged() {
    // The move is the mistake to report; the range of a key about to be moved
    // is not. Otherwise a file migrated by halves loses the useful message.
    let message = error("[preview]\njpeg_quality = 101\n");
    assert!(
        message.contains("score.jpeg_quality"),
        "the move has to be named first: {message}"
    );
}

#[test]
fn a_value_the_flags_would_reject_is_fatal() {
    let cases = [
        ("[score]\nmax_edge = 0\n", "max_edge"),
        ("[score]\njpeg_quality = 0\n", "jpeg_quality"),
        ("[score]\njpeg_quality = 101\n", "jpeg_quality"),
        ("[group]\nmax_edge = 0\n", "group.max_edge"),
        ("[group]\njpeg_quality = 0\n", "group.jpeg_quality"),
        ("[group]\njpeg_quality = 101\n", "group.jpeg_quality"),
        ("[server]\ntimeout = 0\n", "timeout"),
        ("[server]\nmodel = \"  \"\n", "model"),
        ("[server]\nendpoint = \"\"\n", "endpoint"),
        ("[server]\npath = \"\"\n", "server.path"),
        ("[server]\npath = \"v1/systemone\"\n", "server.path"),
        ("[server]\npath = \" /v1/systemone\"\n", "server.path"),
        ("[server]\npath = \"/v1/{frame}\"\n", "server.path"),
        ("[server]\npath = \"/{model}/{frame}\"\n", "server.path"),
        ("[server]\nselector = \"\"\n", "server.selector"),
        ("[server]\nselector = \"  \"\n", "server.selector"),
        ("[server]\ntoken = \"\"\n", "server.token"),
        ("[server]\ntoken = \"  \"\n", "server.token"),
        ("[server]\ntoken_env = \"\"\n", "server.token_env"),
        (
            "[server]\ntoken_env = \"CLOUDFLARE TOKEN\"\n",
            "server.token_env",
        ),
        (
            "[server]\ntoken_env = \"CLOUDFLARE=TOKEN\"\n",
            "server.token_env",
        ),
        ("[group]\ngap = 0.0\n", "group.gap"),
        ("[group]\ngap = -1.0\n", "group.gap"),
        ("[group]\ngap = nan\n", "group.gap"),
        ("[group]\ngap = inf\n", "group.gap"),
        ("[group]\nsimilarity = 1.5\n", "group.similarity"),
        ("[group]\nsimilarity = -0.1\n", "group.similarity"),
        ("[group]\nsimilarity = nan\n", "group.similarity"),
        ("[group]\nexposure_stops = -1.0\n", "group.exposure_stops"),
        ("[group]\nexposure_stops = nan\n", "group.exposure_stops"),
        ("[group]\nexposure_stops = inf\n", "group.exposure_stops"),
        ("[group]\ncull_jobs = 0\n", "group.cull_jobs"),
        ("[group]\nmax_images = 0\n", "group.max_images"),
        ("[group]\nmax_images = 1\n", "group.max_images"),
        ("[group]\nmax_images = 21\n", "group.max_images"),
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
fn a_comparison_cap_outside_two_to_twenty_states_the_range_and_why() {
    // The range is not a guess to be rounded: a request carrying one frame is
    // not a comparison, and 20 is the batch the token budget was measured at.
    for text in [
        "[group]\nmax_images = 0\n",
        "[group]\nmax_images = 1\n",
        "[group]\nmax_images = 21\n",
    ] {
        let message = error(text);
        assert!(
            message.contains("between 2 and 20"),
            "a range mistake must say what the range is: {message}"
        );
        assert!(
            message.contains("one image is not a comparison"),
            "and why the floor is where it is: {message}"
        );
    }
    for accepted in [2, keeper::MAX] {
        let resolved = resolved(&format!("[group]\nmax_images = {accepted}\n"));
        assert_eq!(
            resolved.group_max_images,
            Some(accepted),
            "{accepted} is inside the range and is carried as written"
        );
    }
}

#[test]
fn a_jpeg_quality_outside_one_to_one_hundred_states_the_range() {
    for text in [
        "[score]\njpeg_quality = 101\n",
        "[group]\njpeg_quality = 101\n",
    ] {
        let message = error(text);
        assert!(
            message.contains("1") && message.contains("100"),
            "a range mistake must say what the range is: {message}"
        );
    }
}

#[test]
fn an_address_with_no_path_is_a_config_like_any_other() {
    // The address is posted to as written: `http://127.0.0.1:9000` means `/`, and
    // a file is not refused for lacking a path. It is simply not an oMLX decision
    // path, so `models` will not read it.
    let effective = resolved("[server]\nendpoint = \"http://127.0.0.1:9000\"\n").settle(None);
    assert_eq!(effective.endpoint, "http://127.0.0.1:9000");
    assert!(
        !effective.asks_omlx(),
        "`/` is not the path oMLX serves a decision on"
    );
}

#[test]
fn a_retired_path_key_names_where_it_went() {
    // ADR 0019 folded the path into the address. The key is legal TOML, so the
    // refusal has to be a sentence that names the replacement, not "unknown
    // field" over a key the photographer copied from an older README.
    let message = error("[server]\npath = \"/v1/systemone\"\n");
    assert!(
        message.contains("server.endpoint") && message.contains("/v1/systemone"),
        "the refusal must name the key that replaces it: {message}"
    );
}

#[test]
fn a_retired_selector_key_names_where_it_went() {
    let message = error("[server]\nselector = \"clef-flash\"\n");
    assert!(
        message.contains("server.model") && message.contains("endpoint"),
        "the refusal must say which two values replace it: {message}"
    );
}

#[test]
fn an_address_that_needs_a_model_is_legal_in_a_file() {
    // `endpoint = "…/{model}"` with no model is fatal when the request is built,
    // not here: `--model` can supply the model for one run, and a file cannot
    // know that.
    let resolved = resolved("[server]\nendpoint = \"http://host/v1/{model}\"\n");
    assert_eq!(resolved.endpoint.as_deref(), Some("http://host/v1/{model}"));
    assert_eq!(
        resolved.model, None,
        "the file said nothing about the model, and that is allowed"
    );
}

#[test]
fn a_token_env_says_that_it_names_a_variable() {
    let message = error("[server]\ntoken_env = \"CLOUDFLARE TOKEN\"\n");
    assert!(
        message.contains("environment variable"),
        "a whitespace-bearing name has to be told what the key is for: {message}"
    );
}

#[test]
fn the_retired_wire_keys_are_refused_before_their_values_are_judged() {
    // A selector is refused for being gone, not for being empty: the key that
    // replaces it is `model`, and the remedy names both halves.
    let message = error("[server]\nselector = \"\"\n");
    assert!(
        message.contains("server.model") && message.contains("endpoint"),
        "a retired key has a remedy, and the message must give it: {message}"
    );
}

/// An injected environment: the lookup a resolution reads, over fixed pairs,
/// so a test never touches a process variable another test could race.
fn environment<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_string())
    }
}

#[test]
fn no_token_source_resolves_to_no_token() {
    let effective = Resolved::default().settle(None);
    let token = effective
        .resolve_token(|name| panic!("nothing should be looked up, and {name} was"))
        .expect("no token source is not a failure");
    assert_eq!(token, None, "no key means no header, as before");
}

#[test]
fn the_environment_wins_over_a_literal_token() {
    let effective =
        resolved("[server]\ntoken_env = \"CLOUDFLARE_API_TOKEN\"\ntoken = \"a-literal-secret\"\n")
            .settle(None);
    let token = effective
        .resolve_token(environment(&[(
            "CLOUDFLARE_API_TOKEN",
            "from-the-environment",
        )]))
        .expect("a set variable resolves");
    assert_eq!(
        token,
        Some(Secret::new("from-the-environment")),
        "the variable wins, because a literal travels with the shoot folder"
    );
}

#[test]
fn a_variable_with_stray_whitespace_is_trimmed() {
    // Shell quoting leaves whitespace a photographer cannot see, and a server
    // answers `Bearer  abc ` with 401 per frame. The literal is left as written:
    // a file's value is one its author can read.
    let effective = resolved("[server]\ntoken_env = \"CLOUDFLARE_API_TOKEN\"\n").settle(None);
    let token = effective
        .resolve_token(environment(&[(
            "CLOUDFLARE_API_TOKEN",
            "  from-the-environment  ",
        )]))
        .expect("a padded variable is still a variable");
    assert_eq!(token, Some(Secret::new("from-the-environment")));
}

#[test]
fn an_unset_or_empty_variable_falls_back_to_the_literal() {
    let effective =
        resolved("[server]\ntoken_env = \"CLOUDFLARE_API_TOKEN\"\ntoken = \"a-literal-secret\"\n")
            .settle(None);
    for pairs in [
        vec![],
        vec![("CLOUDFLARE_API_TOKEN", "")],
        vec![("CLOUDFLARE_API_TOKEN", "   ")],
        vec![("SOMETHING_ELSE", "not-the-one-named")],
    ] {
        let token = effective
            .resolve_token(environment(&pairs))
            .expect("a literal beside the name is enough");
        assert_eq!(
            token,
            Some(Secret::new("a-literal-secret")),
            "with nothing in {pairs:?}, the literal is what is sent"
        );
    }
}

#[test]
fn a_named_variable_with_nothing_behind_it_is_fatal_and_names_it() {
    let effective = resolved("[server]\ntoken_env = \"CLOUDFLARE_API_TOKEN\"\n").settle(None);
    let message = effective
        .resolve_token(environment(&[("CLOUDFLARE_API_TOKEN", "")]))
        .expect_err("an empty variable with no literal describes no request")
        .to_string();
    assert!(
        message.contains("CLOUDFLARE_API_TOKEN"),
        "the message has to name the variable to export: {message}"
    );
    assert!(
        message.contains("[server] token is empty"),
        "and the key it falls back to: {message}"
    );
}

#[test]
fn a_settled_token_is_not_printed_by_the_effective_configs_debug() {
    let effective = resolved("[server]\ntoken = \"a-literal-secret\"\n").settle(None);
    let rendered = format!("{effective:#?}");
    assert!(
        !rendered.contains("a-literal-secret"),
        "a run's config is printed in diagnostics; the token may not be: {rendered}"
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
        "[score]\nrubric = \"/etc/rubrics/mine.toml\"\njson_scores = \"/tmp/answers.json\"\n",
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
        score_max_edge: Some(1568),
        model: Some("clef-flash-oQ4e".to_string()),
        json_scores: Some(PathBuf::from("elsewhere.json")),
        group_cull_jobs: Some(1),
        ..Resolved::default()
    };
    let effective = file.override_with(flags).settle(None);
    assert_eq!(
        effective.score_params.max_edge, 1568,
        "--max-edge beat the file"
    );
    assert_eq!(
        effective.cull_jobs, 1,
        "--cull-jobs 1 beat the file's degree, for one gentler run"
    );
    assert_eq!(effective.model.as_deref(), Some("clef-flash-oQ4e"));
    assert_eq!(
        effective.outputs.json_scores,
        Some(PathBuf::from("elsewhere.json"))
    );
    assert_eq!(
        effective.score_params.jpeg_quality, 72,
        "a flag that said nothing leaves the file's value alone"
    );
    assert_eq!(
        effective.thresholds,
        Thresholds {
            gap: std::time::Duration::from_secs_f64(0.75),
            similarity: 0.96,
            exposure_stops: 1.5,
        },
        "and a key no flag touched keeps the file's number"
    );
}

#[test]
fn a_groups_flag_wins_over_the_file() {
    let file = resolved(EVERY_KEY);
    let flags = Resolved {
        recursive: Some(false),
        group_gap: Some(1.25),
        ..Resolved::default()
    };
    let effective = file.override_with(flags).settle(None);
    assert!(!effective.recursive, "--recursive false beat the file");
    assert_eq!(
        effective.thresholds.gap,
        std::time::Duration::from_secs_f64(1.25)
    );
    assert_eq!(
        effective.thresholds.similarity, 0.96,
        "a flag that said nothing leaves the file's value alone"
    );
}

#[test]
fn the_comparison_has_its_own_preview_pair() {
    let effective = resolved(EVERY_KEY).settle(None);
    assert_eq!(
        effective.score_params,
        Params {
            max_edge: 1024,
            jpeg_quality: 72,
        },
        "[score] is the pair a rating sends"
    );
    assert_eq!(
        effective.group_params,
        Params {
            max_edge: 700,
            jpeg_quality: 60,
        },
        "[group]'s pair is the pair a comparison sends, and the two never merge"
    );
}

#[test]
fn a_scores_preview_flag_leaves_the_comparisons_pair_alone() {
    let file = resolved(EVERY_KEY);
    let flags = Resolved {
        score_max_edge: Some(1568),
        score_jpeg_quality: Some(88),
        ..Resolved::default()
    };
    let effective = file.override_with(flags).settle(None);
    assert_eq!(
        effective.score_params,
        Params {
            max_edge: 1568,
            jpeg_quality: 88,
        }
    );
    assert_eq!(
        effective.group_params,
        Params {
            max_edge: 700,
            jpeg_quality: 60,
        },
        "--max-edge is a rating's cap and never reaches the cull"
    );
}

#[test]
fn a_culls_preview_flag_leaves_the_scores_pair_alone() {
    let file = resolved(EVERY_KEY);
    let flags = Resolved {
        group_max_edge: Some(640),
        group_jpeg_quality: Some(50),
        ..Resolved::default()
    };
    let effective = file.override_with(flags).settle(None);
    assert_eq!(
        effective.group_params,
        Params {
            max_edge: 640,
            jpeg_quality: 50,
        },
        "groups --max-edge beat the file's comparison edge"
    );
    assert_eq!(
        effective.score_params,
        Params {
            max_edge: 1024,
            jpeg_quality: 72,
        },
        "and a rating's pair is untouched by a cull's flag"
    );
}

#[test]
fn a_whole_number_is_a_threshold_like_any_other() {
    let resolved = resolved("[group]\ngap = 2\nexposure_stops = 1\n");
    assert_eq!(resolved.group_gap, Some(2.0), "`gap = 2` is two seconds");
    assert_eq!(
        resolved.group_exposure_stops,
        Some(1.0),
        "and `exposure_stops = 1` is one stop"
    );
}

#[test]
fn the_file_wins_over_the_built_in_default() {
    let effective = resolved(EVERY_KEY).settle(None);
    assert_eq!(effective.score_params.max_edge, 1024);
    assert_eq!(effective.score_params.jpeg_quality, 72);
    assert_eq!(effective.timeout, 300);
    assert_eq!(
        effective.endpoint,
        "http://192.168.1.9:8000/v1/systemone/{model}"
    );
    assert!(
        !effective.truncate,
        "and a file that turns oMLX's field off keeps it off"
    );
    assert_eq!(
        effective.model.as_deref(),
        Some("clef-27b-oQ4e"),
        "the model is what the body says and what {{model}} becomes"
    );
    assert_eq!(effective.token_env.as_deref(), Some("CLOUDFLARE_API_TOKEN"));
    assert_eq!(
        effective.token.as_ref().map(Secret::as_str),
        Some("a-literal-secret")
    );
    assert!(!effective.dedupe);
    assert!(effective.recursive);
    assert_eq!(
        effective.thresholds,
        Thresholds {
            gap: std::time::Duration::from_secs_f64(0.75),
            similarity: 0.96,
            exposure_stops: 1.5,
        }
    );
    assert_eq!(
        effective.shoot_context.as_deref(),
        Some("night street, pushed two stops")
    );
    assert_eq!(
        effective.cull_jobs, 2,
        "the file's degree is what a run with no --cull-jobs uses"
    );
    assert_eq!(
        effective.max_images, 4,
        "and the file's cap is what a run's comparisons carry"
    );
}

#[test]
fn an_empty_source_settles_to_the_specs_defaults() {
    let effective = Resolved::default().settle(None);
    assert_eq!(
        effective.endpoint, "http://127.0.0.1:8000/v1/systemone",
        "an absent address is the one oMLX serves a decision model on"
    );
    assert_eq!(
        effective.model, None,
        "no alias means the rubric's own alias, which is part of its hash"
    );
    assert_eq!(effective.token_env, None, "and no variable is looked up");
    assert_eq!(effective.token, None, "and no token is sent");
    assert!(effective.truncate, "an absent truncate is oMLX's field");
    assert_eq!(effective.timeout, 120);
    assert_eq!(effective.score_params, Params::default());
    assert_eq!(
        effective.group_params,
        Params {
            max_edge: keeper::EDGE,
            jpeg_quality: Params::DEFAULT_JPEG_QUALITY,
        },
        "a comparison's default edge is the measured 512, not a rating's 1568"
    );
    assert!(!effective.recursive);
    assert_eq!(effective.outputs.json_scores, None, "no report path at all");
    assert_eq!(effective.rubric, None);
    assert!(effective.dedupe);
    assert_eq!(effective.thresholds, Thresholds::default());
    assert_eq!(
        effective.cull_jobs,
        imagecull_core::cull::DEFAULT_JOBS,
        "an absent degree is the cull's own default, not a second copy of it"
    );
    assert_eq!(
        effective.max_images,
        keeper::MAX,
        "an absent cap is the measured batch of 20, not a second copy of it"
    );
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
fn a_server_nobody_named_still_posts_where_it_always_did() {
    // The default address is oMLX's own: the origin constant plus the path
    // constant, which is what keeps the default from drifting away from the
    // path `is_omlx` recognises.
    let effective = Resolved::default().settle(None);
    assert_eq!(
        effective.endpoint,
        imagecull_core::systemone::DEFAULT_ENDPOINT
    );
    assert_eq!(
        imagecull_core::systemone::DEFAULT_ENDPOINT,
        format!(
            "{}{}",
            imagecull_core::omlx::DEFAULT_ENDPOINT,
            imagecull_core::systemone::PATH
        ),
        "the default address is oMLX on its port, on its decision path"
    );
    assert_eq!(effective.model, None, "and no model at all");
}

#[test]
fn only_the_omlx_path_is_the_one_models_can_read() {
    // `models` reads oMLX's own probes, and no other server has them. The
    // question is asked of the address, so a server that keeps oMLX's fixed path
    // is oMLX to that command and a server that puts its model in the path is
    // not.
    assert!(
        Resolved::default().settle(None).asks_omlx(),
        "a file that names no address posts to oMLX's"
    );
    assert!(
        resolved("[server]\nendpoint = \"http://127.0.0.1:9000/v1/systemone\"\n")
            .settle(None)
            .asks_omlx(),
        "another port on oMLX's own path is still oMLX"
    );
    assert!(
        !resolved("[server]\nendpoint = \"https://host/ai/run/{model}\"\n")
            .settle(None)
            .asks_omlx(),
        "an address with a model in it is another server's addressing"
    );
    assert!(
        !resolved("[server]\nendpoint = \"http://127.0.0.1:9000/v1/decisions\"\n")
            .settle(None)
            .asks_omlx(),
        "and so is any fixed path that is not oMLX's"
    );
}

#[test]
fn a_key_no_flag_sets_survives_the_flags() {
    // No flag sets these six, so `override_with` must project them: a flag
    // layer that says nothing may not quietly drop the file's wire or its cap.
    let effective = resolved(EVERY_KEY)
        .override_with(Resolved {
            timeout: Some(30),
            ..Resolved::default()
        })
        .settle(None);
    assert_eq!(
        effective.endpoint,
        "http://192.168.1.9:8000/v1/systemone/{model}"
    );
    assert_eq!(effective.model.as_deref(), Some("clef-27b-oQ4e"));
    assert_eq!(effective.token_env.as_deref(), Some("CLOUDFLARE_API_TOKEN"));
    assert_eq!(
        effective.token.as_ref().map(Secret::as_str),
        Some("a-literal-secret")
    );
    assert!(!effective.truncate);
    assert_eq!(
        effective.max_images, 4,
        "and the file's comparison cap is not a flag's to lose"
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
        resolved("[score]\nrubric = \"holiday\"\n").rubric,
        Some(RubricSource::Builtin(Builtin::Holiday)),
        "a bare name is a built-in, exactly as on the command line"
    );
    assert_eq!(
        resolved("[score]\nrubric = \"rubrics/mine.toml\"\n").rubric,
        Some(RubricSource::File(PathBuf::from("rubrics/mine.toml")))
    );
}

#[test]
fn a_name_is_not_anchored_to_the_config_files_directory() {
    let base = Path::new("/Volumes/Shoot A");
    let file = ConfigFile::from_toml("[score]\nrubric = \"holiday\"\n").expect("a valid config");
    assert_eq!(
        file.resolve(Some(base)).rubric,
        Some(RubricSource::Builtin(Builtin::Holiday)),
        "there is no path to anchor: a name is a name wherever the file sits"
    );
}

#[test]
fn a_blank_rubric_is_fatal_and_lists_the_built_ins() {
    let message = error("[score]\nrubric = \"  \"\n");
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
        "[score]\nmax_edge = 1024\n",
    )
    .expect("write the shoot config");

    let loaded = config::discover(None, &env_in(dir.path()))
        .expect("discovery")
        .expect("the shoot config");

    assert_eq!(loaded.path, dir.path().join(config::SHOOT_CONFIG));
    assert_eq!(loaded.resolve().score_max_edge, Some(1024));
}

#[test]
fn an_explicit_config_wins_over_the_shoot_config() {
    let dir = TempDir::new().expect("a temp dir");
    fs::write(
        dir.path().join(config::SHOOT_CONFIG),
        "[score]\nmax_edge = 1024\n",
    )
    .expect("write the shoot config");
    fs::write(dir.path().join("mine.toml"), "[score]\nmax_edge = 800\n").expect("write");

    let loaded = config::discover(Some(Path::new("mine.toml")), &env_in(dir.path()))
        .expect("discovery")
        .expect("the explicit config");

    assert_eq!(loaded.path, dir.path().join("mine.toml"));
    assert_eq!(loaded.resolve().score_max_edge, Some(800));
}

#[test]
fn an_explicit_config_that_does_not_exist_is_fatal_not_a_fall_back() {
    let dir = TempDir::new().expect("a temp dir");
    fs::write(
        dir.path().join(config::SHOOT_CONFIG),
        "[score]\nmax_edge = 1024\n",
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
        "[score]\nmax_edge = 1024\n",
    )
    .expect("write the shoot config");
    write_user_config(user.path(), "[score]\nmax_edge = 800\n");

    let env = Env {
        cwd: dir.path().to_path_buf(),
        xdg_config_home: Some(user.path().to_path_buf()),
        home: None,
    };
    let loaded = config::discover(None, &env)
        .expect("discovery")
        .expect("a config");

    assert_eq!(
        loaded.resolve().score_max_edge,
        Some(1024),
        "one file wins whole; they never layer"
    );
}

#[test]
fn the_user_config_is_found_when_no_shoot_config_exists() {
    let dir = TempDir::new().expect("a temp dir");
    let user = TempDir::new().expect("a temp config home");
    write_user_config(user.path(), "[score]\nmax_edge = 800\n");

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
    assert_eq!(loaded.resolve().score_max_edge, Some(800));
}

#[test]
fn the_user_config_falls_back_to_dot_config_under_home() {
    let dir = TempDir::new().expect("a temp dir");
    let home = TempDir::new().expect("a temp home");
    write_user_config(&home.path().join(".config"), "[score]\nmax_edge = 640\n");

    let env = Env {
        cwd: dir.path().to_path_buf(),
        xdg_config_home: None,
        home: Some(home.path().to_path_buf()),
    };
    let loaded = config::discover(None, &env)
        .expect("discovery")
        .expect("a config");

    assert_eq!(loaded.resolve().score_max_edge, Some(640));
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
    fs::write(&path, "[score]\nmax_edeg = 1024\n").expect("write");

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

#[test]
fn the_workers_ai_recipe_in_the_example_is_a_config_that_parses() {
    // The example ends with a Workers AI recipe a photographer pastes over their
    // account id: commented out, so the test above never sees it. One key
    // renamed in the loader and the recipe rots into a file imagecull refuses —
    // discovered after the photographer has filled in their account.
    let recipe = recipe_block(include_str!("../../../imagecull.toml.example"));
    let effective = ConfigFile::from_toml(&recipe)
        .expect("the example's recipe must be a config imagecull accepts")
        .resolve(None)
        .settle(None);

    assert_eq!(
        effective.endpoint,
        "https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/ai/run/@cf/cloudflare/{model}",
        "the id's own prefix is part of the address"
    );
    assert_eq!(
        effective.model.as_deref(),
        Some("clef-flash"),
        "the name fills {{model}} and is the body's model"
    );
    assert_eq!(effective.token_env.as_deref(), Some("CLOUDFLARE_API_TOKEN"));
    assert!(
        !effective.truncate,
        "Workers AI's schema lists no truncate, so the recipe must not send one"
    );
    assert_eq!(
        effective.max_images, 4,
        "Workers AI takes four images a request"
    );
    assert_eq!(effective.score_params.max_edge, 1024);
    assert!(
        !effective.asks_omlx(),
        "the recipe addresses another server"
    );
}

#[test]
fn the_readme_recipe_and_the_example_recipe_describe_the_same_server() {
    // The README repeats the recipe so a photographer need not open a second
    // file, which makes it the copy that can rot without a test noticing: the
    // one above only ever reads the example.
    let readme = fenced_recipe(include_str!("../../../README.md"));
    let example = recipe_block(include_str!("../../../imagecull.toml.example"));
    let settle = |text: &str| {
        ConfigFile::from_toml(text)
            .expect("a recipe a reader pastes must be a config imagecull accepts")
            .resolve(None)
            .settle(None)
    };

    assert_eq!(
        settle(&readme),
        settle(&example),
        "the two copies of the Workers AI recipe must describe the same server"
    );
}

/// The first fenced `toml` block in a document that names an account id: the
/// Workers AI recipe a README shows whole, rather than commented out.
fn fenced_recipe(document: &str) -> String {
    for block in document.split("```toml").skip(1) {
        let body = block.split("```").next().unwrap_or_default();
        if body.contains("<ACCOUNT_ID>") {
            return body.trim_start_matches('\n').to_string();
        }
    }
    panic!("no ```toml block naming <ACCOUNT_ID> in the document");
}

/// The commented recipe an example file carries between its two rules, with the
/// comment markers taken off: the config a photographer would paste.
fn recipe_block(example: &str) -> String {
    const BEGINS: &str = "recipe begins";
    const ENDS: &str = "recipe ends";
    let mut recipe = String::new();
    let mut inside = false;
    for line in example.lines() {
        if line.contains(ENDS) {
            break;
        }
        if inside {
            let text = line
                .strip_prefix("# ")
                .or_else(|| line.strip_prefix('#'))
                .unwrap_or(line);
            recipe.push_str(text);
            recipe.push('\n');
        } else if line.contains(BEGINS) {
            inside = true;
        }
    }
    assert!(
        inside,
        "the example must carry a '{BEGINS}' … '{ENDS}' recipe"
    );
    recipe
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
