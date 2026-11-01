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
