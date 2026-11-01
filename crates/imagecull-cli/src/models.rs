//! `imagecull models`: what is this server actually serving?
//!
//! `/v1/models` alone cannot answer the question that matters — whether a
//! listed alias is a decision model or just a chat model — so this command
//! joins it with `/v1/models/status` and marks the decision models. On a server
//! too old to have that endpoint, it says so instead of pretending.

use imagecull_core::ExitCode;
use imagecull_core::omlx::{Catalog, MIN_DECISION_VERSION};

use crate::cli::ServerArgs;

/// Run the command and return the process exit code.
pub fn run(args: &ServerArgs) -> u8 {
    let client = match args.client() {
        Ok(client) => client,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::Fatal.code();
        }
    };

    // The version is a nicety here: `models` is still useful when `/health`
    // will not parse, as long as the listings do.
    let version = client.health().ok().and_then(|health| health.version);

    match client.catalog() {
        Ok(catalog) => {
            print!(
                "{}",
                render(
                    &catalog,
                    version.as_deref(),
                    client.endpoint().as_str(),
                    args.model()
                )
            );
            ExitCode::Ok.code()
        }
        Err(error) => {
            eprintln!(
                "error: {} — run imagecull doctor for a diagnosis",
                error.message()
            );
            ExitCode::Fatal.code()
        }
    }
}

/// Render the listing. Pure, so it can be tested without a server.
pub fn render(
    catalog: &Catalog,
    version: Option<&str>,
    endpoint: &str,
    configured: &str,
) -> String {
    let mut out = String::new();
    let server = match version {
        Some(version) => format!("oMLX {version}"),
        None => "oMLX".to_string(),
    };
    let served = catalog.models().len();
    let decision = catalog.decision_count();
    out.push_str(&format!(
        "{server} at {endpoint} — {} {} served, {} {}\n",
        served,
        plural(served, "model", "models"),
        decision,
        plural(decision, "decision model", "decision models"),
    ));

    if served == 0 {
        out.push_str("\n  no models served — load a Clef or Clef-Flash checkpoint in oMLX\n");
        return out;
    }

    let width = catalog
        .models()
        .iter()
        .map(|model| model.alias.len())
        .max()
        .unwrap_or(0);
    out.push('\n');
    for model in catalog.models() {
        let mark = if model.is_decision() { '*' } else { ' ' };
        let configured = if model.alias == configured {
            "  (configured)"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {mark} {alias:<width$}  {kind}{configured}\n",
            alias = model.alias,
            kind = model.kind.as_str(),
            width = width,
        ));
    }

    if decision > 0 {
        out.push_str(
            "\n* a decision model: answers /v1/systemone, which is what imagecull scores with\n",
        );
    }
    if !catalog.types_reported() {
        out.push_str(&format!(
            "\n! this server does not report model types (GET /v1/models/status is missing), \
             so none of the above could be marked — upgrade oMLX to {MIN_DECISION_VERSION} or \
             later to score photos\n"
        ));
    }
    out
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

#[cfg(test)]
mod tests {
    use imagecull_core::omlx::{ModelKind, ServedModel};

    use super::*;

    fn model(alias: &str, kind: ModelKind, model_type: Option<&str>) -> ServedModel {
        ServedModel {
            alias: alias.to_string(),
            kind,
            model_type: model_type.map(str::to_string),
        }
    }

    #[test]
    fn decision_models_are_marked_and_the_configured_one_named() {
        let catalog = Catalog::new(
            vec![
                model("clef-flash-oQ4e", ModelKind::Decision, Some("decision")),
                model("qwen3.5-4b", ModelKind::Chat, Some("vlm")),
            ],
            true,
        );
        let rendered = render(
            &catalog,
            Some("0.7.1.dev1"),
            "http://127.0.0.1:8000",
            "clef-flash-oQ4e",
        );

        assert!(rendered.contains(
            "oMLX 0.7.1.dev1 at http://127.0.0.1:8000 — 2 models served, 1 decision model"
        ));
        assert!(rendered.contains("* clef-flash-oQ4e"));
        assert!(rendered.contains("decision  (configured)"));
        assert!(rendered.contains("qwen3.5-4b"));
        assert!(rendered.contains("chat"));
        assert!(rendered.contains("* a decision model"));
        assert!(!rendered.contains("does not report model types"));
    }

    #[test]
    fn an_old_server_says_types_are_missing_rather_than_guessing() {
        let catalog = Catalog::new(
            vec![model("clef-flash-oQ4e", ModelKind::Unreported, None)],
            false,
        );
        let rendered = render(&catalog, None, "http://127.0.0.1:8000", "clef-flash-oQ4e");

        assert!(
            rendered
                .starts_with("oMLX at http://127.0.0.1:8000 — 1 model served, 0 decision models")
        );
        assert!(rendered.contains("clef-flash-oQ4e"));
        assert!(rendered.contains("unknown  (configured)"));
        assert!(rendered.contains("/v1/models/status is missing"));
        assert!(rendered.contains(MIN_DECISION_VERSION));
        assert!(!rendered.contains("* a decision model"));
    }

    #[test]
    fn an_empty_server_explains_what_to_load() {
        let catalog = Catalog::new(vec![], true);
        let rendered = render(
            &catalog,
            Some("0.7.1.dev1"),
            "http://127.0.0.1:8000",
            "clef-flash-oQ4e",
        );
        assert!(rendered.contains("0 models served, 0 decision models"));
        assert!(rendered.contains("no models served — load a Clef or Clef-Flash checkpoint"));
    }
}
