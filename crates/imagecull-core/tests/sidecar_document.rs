//! The XMP sidecar document: documents in, documents out, no filesystem.
//!
//! Every assertion here is about text a photographer or another application will
//! read: the properties we own, the bytes we do not own, and the refusals. The
//! expected documents are written out by hand from the spec rather than derived
//! from the code, so a change that reorders or re-quotes foreign XML cannot pass
//! by agreeing with itself.

use imagecull_core::ErrorKind;
use imagecull_core::sidecar::{self, Properties};

/// The properties of one frame, spelled the way a run would hand them over.
fn properties() -> Properties {
    Properties {
        rating: 4,
        conf_stars: 0.63,
        model: "clef-flash-oQ4e".to_string(),
        rubric_hash: "9f3c1a77".to_string(),
        scored_at: "2026-11-08T10:14:02Z".to_string(),
        version: "0.1.0".to_string(),
    }
}

/// A sidecar another application wrote: mixed quote styles, an attribute order
/// nobody would choose, a develop setting, a keyword bag, and a rating of 3.
const FOREIGN: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Adobe XMP Core 7.0\">\n\
 \x20<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
 \x20 <rdf:Description rdf:about=\"\"\n\
 \x20   xmlns:xmp='http://ns.adobe.com/xap/1.0/'\n\
 \x20   xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"\n\
 \x20   crs:Exposure2012=\"+0.35\"\n\
 \x20   xmp:Rating=\"3\">\n\
 \x20  <dc:subject xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
 \x20   <rdf:Bag><rdf:li>harbour</rdf:li></rdf:Bag>\n\
 \x20  </dc:subject>\n\
 \x20 </rdf:Description>\n\
 \x20</rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>";

#[test]
fn a_foreign_document_keeps_every_byte_it_does_not_own() {
    let document = sidecar::render(Some(FOREIGN), &properties()).expect("a merged document");

    assert_eq!(
        document,
        FOREIGN
            .replace("xmp:Rating=\"3\"", "xmp:Rating=\"4\"")
            .replace(
                "xmp:Rating=\"4\">",
                "xmp:Rating=\"4\" \
                 xmlns:imagecull=\"https://ns.imagecull.app/1.0/\" \
                 imagecull:confStars=\"0.63\" \
                 imagecull:model=\"clef-flash-oQ4e\" \
                 imagecull:rubricHash=\"9f3c1a77\" \
                 imagecull:scoredAt=\"2026-11-08T10:14:02Z\" \
                 imagecull:version=\"0.1.0\">"
            ),
        "the develop setting, the keywords, the quotes and the attribute order all survive"
    );
}

#[test]
fn a_document_without_a_rating_gets_one_and_the_namespaces_it_needs() {
    let existing = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF \
                    xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
                    rdf:about=\"\" crs:Exposure2012=\"+0.35\" \
                    xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"/></rdf:RDF></x:xmpmeta>";

    let document = sidecar::render(Some(existing), &properties()).expect("a merged document");

    assert_eq!(
        document,
        existing.replace(
            "xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"/>",
            "xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\" \
             xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
             xmlns:imagecull=\"https://ns.imagecull.app/1.0/\" \
             xmp:Rating=\"4\" \
             imagecull:confStars=\"0.63\" \
             imagecull:model=\"clef-flash-oQ4e\" \
             imagecull:rubricHash=\"9f3c1a77\" \
             imagecull:scoredAt=\"2026-11-08T10:14:02Z\" \
             imagecull:version=\"0.1.0\"/>"
        )
    );
}

#[test]
fn merging_our_own_document_again_changes_nothing() {
    let once = sidecar::render(None, &properties()).expect("a new document");
    let twice = sidecar::render(Some(&once), &properties()).expect("a merged document");

    assert_eq!(twice, once, "a re-run must not grow the document");
}

#[test]
fn a_document_with_nothing_to_write_into_is_refused_rather_than_guessed_at() {
    let existing = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>";

    let error = sidecar::render(Some(existing), &properties()).expect_err("a refusal");

    assert_eq!(error.kind(), Some(ErrorKind::XmpUnreadable));
    assert!(
        error.message().contains("rdf:Description"),
        "the refusal has to say what is missing: {}",
        error.message()
    );
}

#[test]
fn a_model_alias_with_xml_characters_is_escaped() {
    let mut properties = properties();
    properties.model = "clef & \"quoted\"".to_string();

    let document = sidecar::render(None, &properties).expect("a new document");

    assert!(
        document.contains("imagecull:model=\"clef &amp; &quot;quoted&quot;\""),
        "an unescaped value would break the document: {document}"
    );
}

#[test]
fn a_forced_rewrite_replaces_our_properties_without_duplicating_them() {
    let first = sidecar::render(None, &properties()).expect("a new document");
    let mut second = properties();
    second.rating = 2;
    second.model = "clef-27b-oQ4e".to_string();
    second.conf_stars = 0.41;
    second.rubric_hash = "7712be04".to_string();

    let document = sidecar::render(Some(&first), &second).expect("a merged document");

    assert_eq!(
        document,
        first
            .replace("xmp:Rating=\"4\"", "xmp:Rating=\"2\"")
            .replace(
                "imagecull:confStars=\"0.63\"",
                "imagecull:confStars=\"0.41\""
            )
            .replace(
                "imagecull:model=\"clef-flash-oQ4e\"",
                "imagecull:model=\"clef-27b-oQ4e\""
            )
            .replace(
                "imagecull:rubricHash=\"9f3c1a77\"",
                "imagecull:rubricHash=\"7712be04\""
            )
    );
}

#[test]
fn a_greater_than_sign_inside_a_value_does_not_end_the_description_tag() {
    let existing = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:exif=\"http://ns.adobe.com/exif/1.0/\" exif:UserComment=\"1 > 0\"></rdf:Description></rdf:RDF></x:xmpmeta>";

    let document = sidecar::render(Some(existing), &properties()).expect("a merged document");

    assert!(
        document.contains("exif:UserComment=\"1 > 0\""),
        "the quote-aware scan must not stop at a `>` inside a value: {document}"
    );
    assert!(document.contains("xmp:Rating=\"4\""), "{document}");
    assert!(
        document.contains("</rdf:Description>"),
        "the element's body must survive: {document}"
    );
}

#[test]
fn a_frame_with_no_sidecar_yields_a_document_carrying_only_our_properties() {
    let document = sidecar::render(None, &properties()).expect("a new document");

    assert_eq!(
        document,
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"imagecull 0.1.0\">\n\
         \x20<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         \x20 <rdf:Description rdf:about=\"\"\n\
         \x20   xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n\
         \x20   xmlns:imagecull=\"https://ns.imagecull.app/1.0/\"\n\
         \x20   xmp:Rating=\"4\"\n\
         \x20   imagecull:confStars=\"0.63\"\n\
         \x20   imagecull:model=\"clef-flash-oQ4e\"\n\
         \x20   imagecull:rubricHash=\"9f3c1a77\"\n\
         \x20   imagecull:scoredAt=\"2026-11-08T10:14:02Z\"\n\
         \x20   imagecull:version=\"0.1.0\"/>\n\
         \x20</rdf:RDF>\n\
         </x:xmpmeta>\n"
    );
}

/// A verdict on a frame that lost its group, spelled the way a run hands it
/// over: red, and every fact needed to explain the red without the folder.
fn mark(label: Option<&'static str>) -> sidecar::Mark {
    sidecar::Mark {
        label,
        keeper: label.is_none(),
        group_start: "2026-11-02T10:00:03.25".to_string(),
        group_size: 4,
        grouping_hash: "2c7d90ab1f".to_string(),
        pick_by: "comparison".to_string(),
        pick_conf: 0.71,
        picked_at: "2026-11-19T09:31:00Z".to_string(),
        rounds: 1,
        retire_label: false,
    }
}

#[test]
fn a_non_keepers_verdict_writes_the_red_and_the_facts_that_explain_it() {
    let rated = sidecar::render(None, &properties()).expect("a rated document");

    let document =
        sidecar::render_mark(&rated, &mark(Some(sidecar::RED))).expect("a marked document");

    assert_eq!(
        document,
        rated.replace(
            "imagecull:version=\"0.1.0\"/>",
            "imagecull:version=\"0.1.0\" \
             xmp:Label=\"Red\" \
             imagecull:labeledBy=\"imagecull\" \
             imagecull:groupKeeper=\"false\" \
             imagecull:groupStart=\"2026-11-02T10:00:03.25\" \
             imagecull:groupSize=\"4\" \
             imagecull:groupingHash=\"2c7d90ab1f\" \
             imagecull:pickBy=\"comparison\" \
             imagecull:pickConf=\"0.71\" \
             imagecull:pickRounds=\"1\" \
             imagecull:pickedAt=\"2026-11-19T09:31:00Z\"/>"
        ),
        "the verdict joins the rating without disturbing a byte of it"
    );
}

#[test]
fn a_keepers_verdict_leaves_the_red_a_previous_run_wrote_exactly_where_it_was() {
    let rated = sidecar::render(None, &properties()).expect("a rated document");
    let marked =
        sidecar::render_mark(&rated, &mark(Some(sidecar::RED))).expect("a marked document");

    let promoted = sidecar::render_mark(&marked, &mark(None)).expect("a promoted document");

    assert!(
        promoted.contains("xmp:Label=\"Red\""),
        "a label is never removed, not even one this tool wrote: {promoted}"
    );
    assert!(
        promoted.contains("imagecull:labeledBy=\"imagecull\""),
        "and the proof that it was ours survives, so a later run can still explain it: {promoted}"
    );
    assert!(
        promoted.contains("imagecull:groupKeeper=\"true\""),
        "the verdict is not the label: the keeper's groupKeeper is true: {promoted}"
    );
    assert_eq!(
        promoted.matches("imagecull:groupKeeper=").count(),
        1,
        "rewritten in place, not appended twice: {promoted}"
    );
}

#[test]
fn our_own_label_is_replaced_in_place_and_keeps_the_documents_quoting() {
    let existing = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                    <rdf:Description xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                    xmlns:imagecull=\"https://ns.imagecull.app/1.0/\" \
                    xmp:Rating='2' xmp:Label='Red' \
                    imagecull:labeledBy='imagecull'/></x:xmpmeta>";

    let document = sidecar::render_mark(existing, &mark(Some(sidecar::RED))).expect("a document");

    assert!(
        document.contains("xmp:Label='Red'") && document.contains("xmp:Rating='2'"),
        "the quoting and the rating survive: {document}"
    );
    assert!(
        document.contains("imagecull:groupSize=\"4\""),
        "the facts the document did not carry are appended: {document}"
    );
}

#[test]
fn a_rating_write_leaves_a_verdict_beside_it_alone() {
    let marked = sidecar::render_mark(
        &sidecar::render(None, &properties()).expect("a rated document"),
        &mark(Some(sidecar::RED)),
    )
    .expect("a marked document");
    let mut again = properties();
    again.rating = 2;

    let document = sidecar::render(Some(&marked), &again).expect("a re-rated document");

    assert!(document.contains("xmp:Rating=\"2\""), "{document}");
    assert!(
        document.contains("xmp:Label=\"Red\"") && document.contains("imagecull:pickConf=\"0.71\""),
        "re-rating a frame must not erase the verdict that explains its colour: {document}"
    );
}

#[test]
fn a_note_an_older_build_wrote_is_retired_and_nothing_else_is() {
    let rated = sidecar::render(None, &properties()).expect("a document");
    // What ticket 02's build left on a frame the request could not carry: the
    // property is gone from this build, and ours are ours to retire.
    let older = rated.replace(
        "imagecull:version=\"0.1.0\"/>",
        "imagecull:version=\"0.1.0\" imagecull:groupNote=\"not_compared\"/>",
    );

    let document = sidecar::render_mark(&older, &mark(Some(sidecar::RED))).expect("a mark");

    assert!(
        !document.contains("imagecull:groupNote"),
        "an older build's note is not left behind: {document}"
    );
    assert!(document.contains("xmp:Rating=\"4\""), "{document}");
    assert!(document.contains("xmp:Label=\"Red\""), "{document}");
}

#[test]
fn a_verdict_names_how_many_rounds_it_took() {
    let rated = sidecar::render(None, &properties()).expect("a document");
    let mut two_rounds = mark(Some(sidecar::RED));
    two_rounds.rounds = 2;

    let document = sidecar::render_mark(&rated, &two_rounds).expect("a marked document");

    assert!(
        document.contains("imagecull:pickRounds=\"2\""),
        "a group compared in batches says so, so a verdict is never read as one request's \
         certainty: {document}"
    );
}

#[test]
fn a_promoted_keeper_loses_the_red_this_tool_wrote() {
    let rated = sidecar::render(None, &properties()).expect("a rated document");
    let marked =
        sidecar::render_mark(&rated, &mark(Some(sidecar::RED))).expect("a marked document");
    let mut promoted = mark(None);
    promoted.retire_label = true;

    let document = sidecar::render_mark(&marked, &promoted).expect("a promoted document");

    assert!(
        !document.contains("xmp:Label"),
        "the red this tool wrote is gone: a repeatable cull may not leave the new keeper red: \
         {document}"
    );
    assert!(
        !document.contains("imagecull:labeledBy"),
        "and the proof goes with it, so a colour the photographer sets next is theirs: {document}"
    );
    assert!(
        document.contains("imagecull:groupKeeper=\"true\""),
        "the verdict this run reached is still recorded: {document}"
    );
    assert!(document.contains("xmp:Rating=\"4\""), "{document}");
}

#[test]
fn a_red_without_our_proof_survives_a_promotion() {
    let theirs = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                  <rdf:Description xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                  xmp:Rating=\"4\" xmp:Label=\"Red\"/></x:xmpmeta>";
    let mut promoted = mark(None);
    promoted.retire_label = true;

    let document = sidecar::render_mark(theirs, &promoted).expect("a promoted document");

    assert!(
        document.contains("xmp:Label=\"Red\""),
        "a red this tool did not write is never removed, whatever the verdict says: {document}"
    );
    assert!(
        !document.contains("imagecull:labeledBy"),
        "and the run does not claim it: {document}"
    );
}

#[test]
fn a_singleton_retirement_removes_our_red_and_the_verdict_and_keeps_the_rating() {
    let rated = sidecar::render(None, &properties()).expect("a rated document");
    let marked =
        sidecar::render_mark(&rated, &mark(Some(sidecar::RED))).expect("a marked document");

    let document = sidecar::retire_label(&marked).expect("a retired document");

    assert!(!document.contains("xmp:Label"), "{document}");
    assert!(!document.contains("imagecull:labeledBy"), "{document}");
    assert!(
        !document.contains("imagecull:groupKeeper") && !document.contains("imagecull:pickConf"),
        "a frame that is no longer in a group carries no verdict about one: {document}"
    );
    assert!(
        document.contains("xmp:Rating=\"4\"") && document.contains("imagecull:version="),
        "everything the rating owns is untouched: {document}"
    );

    let theirs = marked.replace("imagecull:labeledBy=\"imagecull\" ", "");
    let kept = sidecar::retire_label(&theirs).expect("a kept document");
    assert!(
        kept.contains("xmp:Label=\"Red\""),
        "a red without our proof is the photographer's, group or no group: {kept}"
    );
}

#[test]
fn a_label_that_is_not_ours_to_write_is_never_overwritten() {
    let photographer = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                        <rdf:Description xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                        xmp:Rating=\"4\" xmp:Label=\"Yellow\"/></x:xmpmeta>";

    let label = sidecar::existing_label(photographer).expect("a label");
    assert_eq!(label.value, "Yellow");
    assert!(!label.ours, "no labeledBy, so this tool did not write it");
    assert!(!sidecar::label_is_ours_to_write(Some(photographer)));

    let ours = photographer.replace("xmp:Label=\"Yellow\"", "xmp:Label=\"Red\"");
    let ours = sidecar::render_mark(&ours, &mark(Some(sidecar::RED))).expect("a mark");

    assert!(
        sidecar::label_is_ours_to_write(Some(&ours)),
        "a label this tool wrote is ours to rewrite"
    );
    assert!(
        sidecar::label_is_ours_to_write(None),
        "and a frame with no label at all is the other case we may write"
    );
}
