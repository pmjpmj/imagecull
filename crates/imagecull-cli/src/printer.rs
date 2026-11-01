//! `--print-sidecars`: what the run would write, printed instead of written.
//!
//! The writer is the only thing that knows a sidecar's shape, so the preview is a
//! writer that prints. Nothing touches the shoot folder: no sidecar, no backup, no
//! temp file — and the requests are paid for exactly as a real run would pay them,
//! because the point of the preview is to see what those requests would become.

use std::io::Write;
use std::path::Path;

use imagecull_core::Result;
use imagecull_core::sidecar::{self, Prior, Writer, Written};

/// The writer a preview run uses.
pub struct Printer;

impl Writer for Printer {
    fn write(&self, frame: &Path, document: &str, prior: Prior<'_>) -> Result<Written> {
        let out = block(frame, document, prior);
        // A closed pipe is the reader's decision, not a failed run.
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
        Ok(if prior.text().is_some() {
            Written::Replaced
        } else {
            Written::Created
        })
    }
}

/// One frame's preview: what would be created or updated, and which of our
/// properties would change, with the value that is there now beside it.
///
/// A frame the run judges twice — its rating, then its group's verdict — gets a
/// block for each write, in the order the writes would happen, so the label line
/// is read beside the rating it is attached to.
fn block(frame: &Path, document: &str, prior: Prior<'_>) -> String {
    let target = sidecar::path_for(frame);
    let verb = if prior.text().is_some() {
        "would update"
    } else {
        "would create"
    };
    let mut out = format!("{}  {verb} {}\n", name_of(frame), name_of(&target));
    let before = prior.text().map(sidecar::properties_in).unwrap_or_default();
    let after = sidecar::properties_in(document);
    for (name, value) in &after {
        match before.iter().find(|(was, _)| was == name) {
            Some((_, was)) => out.push_str(&format!("  {name:<22} {value}      (was: {was})\n")),
            None => out.push_str(&format!("  {name:<22} {value}      (was: none)\n")),
        }
    }
    // A property this write removes is not in the document, so it is the one
    // change a preview would otherwise hide: a promotion loses the red, and a
    // reviewer has to see that before it happens.
    for (name, was) in &before {
        if after.iter().all(|(now, _)| now != name) {
            out.push_str(&format!("  {name:<22} (removed)      (was: {was})\n"));
        }
    }
    out.push('\n');
    out
}

/// The last component of a path, as the preview spells it.
fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use imagecull_core::sidecar::{self, Prior, Properties};

    use super::block;

    fn properties(rating: u8) -> Properties {
        Properties {
            rating,
            conf_stars: 0.63,
            model: "clef-flash-oQ4e".to_string(),
            rubric_hash: "9f3c1a77".to_string(),
            scored_at: "2026-11-08T10:14:02Z".to_string(),
            version: "0.1.0".to_string(),
        }
    }

    #[test]
    fn a_new_sidecar_shows_every_property_with_nothing_before_it() {
        let document = sidecar::render(None, &properties(4)).expect("a document");

        let block = block(Path::new("/shoot/DSC_0421.NEF"), &document, Prior::Absent);

        assert_eq!(
            block,
            "DSC_0421.NEF  would create DSC_0421.xmp\n\
             \x20 xmp:Rating             4      (was: none)\n\
             \x20 imagecull:confStars    0.63      (was: none)\n\
             \x20 imagecull:model        clef-flash-oQ4e      (was: none)\n\
             \x20 imagecull:rubricHash   9f3c1a77      (was: none)\n\
             \x20 imagecull:scoredAt     2026-11-08T10:14:02Z      (was: none)\n\
             \x20 imagecull:version      0.1.0      (was: none)\n\n"
        );
    }

    #[test]
    fn an_existing_sidecar_shows_the_rating_it_would_replace() {
        let existing = sidecar::render(None, &properties(3)).expect("a document");
        let document = sidecar::render(Some(&existing), &properties(2)).expect("a document");

        let block = block(
            Path::new("/shoot/DSC_0422.NEF"),
            &document,
            Prior::Earlier(&existing),
        );

        assert!(
            block.starts_with("DSC_0422.NEF  would update DSC_0422.xmp\n"),
            "{block}"
        );
        assert!(
            block.contains("xmp:Rating             2      (was: 3)"),
            "the reviewer has to see what the run would overwrite: {block}"
        );
    }

    /// A verdict on a frame that lost its group, as the run would write one.
    fn mark() -> sidecar::Mark {
        sidecar::Mark {
            label: Some(sidecar::RED),
            keeper: false,
            group_start: "2026-11-02T10:00:00.100".to_string(),
            group_size: 3,
            grouping_hash: "2c7d90ab1f".to_string(),
            pick_by: "comparison".to_string(),
            pick_conf: 0.71,
            picked_at: "2026-11-19T09:31:00Z".to_string(),
            rounds: 1,
            retire_label: false,
        }
    }

    #[test]
    fn a_marked_frame_shows_the_label_line_with_the_colour_that_was_there_before() {
        let rated = sidecar::render(None, &properties(4)).expect("a rated document");
        let marked = sidecar::render_mark(&rated, &mark()).expect("a marked document");

        let block = block(
            Path::new("/shoot/DSC_0421.NEF"),
            &marked,
            Prior::Earlier(&rated),
        );

        assert!(
            block.contains("xmp:Label              Red      (was: none)\n"),
            "the reviewer has to see the colour the cull will filter on: {block}"
        );
        assert!(
            block.contains("imagecull:labeledBy    imagecull      (was: none)"),
            "and that this tool is the one claiming it: {block}"
        );
        assert!(
            block.contains("xmp:Rating             4      (was: 4)"),
            "the rating the label sits beside is unchanged, and the preview says so: {block}"
        );
    }

    #[test]
    fn a_removal_is_shown_with_the_colour_that_was_there_before() {
        let rated = sidecar::render(None, &properties(4)).expect("a rated document");
        let marked = sidecar::render_mark(&rated, &mark()).expect("a marked document");
        let promoted = sidecar::render_mark(
            &marked,
            &sidecar::Mark {
                label: None,
                keeper: true,
                retire_label: true,
                ..mark()
            },
        )
        .expect("a promoted document");

        let block = block(
            Path::new("/shoot/DSC_0421.NEF"),
            &promoted,
            Prior::Earlier(&marked),
        );

        assert!(
            block.contains("xmp:Label              (removed)      (was: Red)\n"),
            "a promotion removes our red, and a reviewer sees it go: {block}"
        );
        assert!(
            block.contains("imagecull:labeledBy    (removed)      (was: imagecull)"),
            "the proof goes with the label: {block}"
        );
    }
}
