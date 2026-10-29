use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, ObjRef, Object, SaveOptions, write_full};

use super::*;
use crate::pages;

fn build(bodies: &[&str]) -> Document {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, b) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{b}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    Document::open(Arc::new(out)).expect("opens")
}

/// One visible page and a hidden template, `More`, whose page holds the text field `Extra`
/// (a field that is its own widget, as forms write it) and has no media box of its own.
fn form() -> Document {
    build(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /Templates 6 0 R >> /AcroForm << /Fields [7 0 R] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 9 0 R >> >> >>",
        "<< /Length 40 >>\nstream\nBT /F1 12 Tf 20 20 Td (Visible) Tj ET\nendstream",
        "<< /Type /Template /Contents 8 0 R /Resources << /Font << /F1 9 0 R >> >> /Annots [7 0 R] >>",
        "<< /Names [(More) 5 0 R] >>",
        "<< /FT /Tx /T (Extra) /V (x) /Subtype /Widget /Rect [20 100 180 120] /P 5 0 R >>",
        "<< /Length 41 >>\nstream\nBT /F1 12 Tf 20 20 Td (Template) Tj ET\nendstream",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ])
}

/// What each page says, through its content stream.
fn texts(doc: &Document) -> Vec<String> {
    pages(doc)
        .unwrap()
        .iter()
        .map(|p| {
            let contents = doc.get(p.obj).as_dict().and_then(|d| d.get(b"Contents").cloned()).expect("contents");
            let Object::Stream(s) = doc.resolve(&contents).as_ref().clone() else { panic!("a stream") };
            let data = s.decoded().unwrap();
            let t = String::from_utf8_lossy(&data).into_owned();
            t.split('(').nth(1).and_then(|x| x.split(')').next()).unwrap_or_default().to_string()
        })
        .collect()
}

/// The fully-qualified names of the form's fields that have a widget on page `page` (0-based).
fn fields_on(doc: &Document, page: usize) -> Vec<String> {
    let page = pages(doc).unwrap()[page].obj;
    let annots = doc.get(page).as_dict().and_then(|d| d.get(b"Annots").cloned()).map(|a| doc.resolve(&a).as_ref().clone());
    let mut out = Vec::new();
    for w in annots.as_ref().and_then(Object::as_array).into_iter().flatten().filter_map(Object::as_ref) {
        out.push(full_name(doc, w));
    }
    out.sort();
    out
}

fn full_name(doc: &Document, node: ObjRef) -> String {
    let mut parts = Vec::new();
    let mut at = Some(node);
    while let Some(r) = at {
        let d = doc.get(r).as_dict().cloned().unwrap_or_default();
        if let Some(t) = d.get(b"T").and_then(|t| doc.resolve(t).as_string().map(|s| s.to_text())) {
            parts.push(t);
        }
        at = d.reference(b"Parent");
    }
    parts.reverse();
    parts.join(".")
}

/// The top-level fields the form lists, by name.
fn listed_fields(doc: &Document) -> Vec<String> {
    let catalog = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let form = doc.resolve(catalog.get(b"AcroForm").unwrap()).as_dict().cloned().unwrap();
    let fields = doc.resolve(form.get(b"Fields").unwrap()).as_array().cloned().unwrap();
    let mut names: Vec<String> = fields.iter().filter_map(Object::as_ref).map(|r| full_name(doc, r)).collect();
    names.sort();
    names
}

fn reopen(doc: &Document) -> Document {
    let bytes = write_full(doc, &SaveOptions::default()).expect("writes");
    let independent = hayro_syntax::Pdf::new(bytes.clone()).expect("another reader opens it");
    let reopened = Document::open(Arc::new(bytes)).expect("reopens");
    assert_eq!(independent.pages().len(), page_count(&reopened).unwrap(), "the readers agree on the pages");
    reopened
}

#[test]
fn hidden_templates_and_named_pages_are_listed() {
    assert_eq!(templates(&form()), [Template { name: "More".into(), hidden: true }]);
    let mut doc = form();
    set_template_hidden(&mut doc, "More", false).unwrap();
    assert_eq!(templates(&doc), [Template { name: "More".into(), hidden: false }]);
}

/// `spawn(1, false, false)`: a copy of the template's page, whose widget joins the template's
/// own field, so the two share the value, as in Acrobat.
#[test]
fn a_spawned_page_without_renaming_shares_the_templates_fields() {
    let mut doc = form();
    assert_eq!(spawn_template(&mut doc, "More", 1, false).unwrap(), 1);
    let doc = reopen(&doc);
    assert_eq!(texts(&doc), ["Visible", "Template"]);
    assert_eq!(fields_on(&doc, 1), ["Extra"]);
    assert_eq!(listed_fields(&doc), ["Extra"], "no second field of the same name");
    let page = pages(&doc).unwrap()[1].obj;
    let media = doc.get(page).as_dict().and_then(|d| d.get(b"MediaBox").cloned());
    assert!(media.is_some(), "a template without a media box takes the first page's");
    assert_eq!(templates(&doc), [Template { name: "More".into(), hidden: true }], "the template stays");
}

/// `spawn(0, true, false)`: the copy's fields are its own, named `P0.More.Extra`.
#[test]
fn a_spawned_page_with_renaming_has_fields_of_its_own() {
    let mut doc = form();
    assert_eq!(spawn_template(&mut doc, "More", 0, true).unwrap(), 0);
    let doc = reopen(&doc);
    assert_eq!(texts(&doc), ["Template", "Visible"]);
    assert_eq!(fields_on(&doc, 0), ["P0.More.Extra"]);
    assert_eq!(listed_fields(&doc), ["Extra", "P0"]);
}

/// A position past the end appends the page.
#[test]
fn spawning_past_the_end_appends() {
    let mut doc = form();
    assert_eq!(spawn_template(&mut doc, "More", 99, false).unwrap(), 1);
    assert_eq!(texts(&reopen(&doc)), ["Visible", "Template"]);
}

/// `hidden = false` moves the template's own page into the document, so its fields have a page;
/// `hidden = true` takes it out again, with what it inherited.
#[test]
fn showing_and_hiding_a_template_moves_its_own_page() {
    let mut doc = form();
    set_template_hidden(&mut doc, "More", false).unwrap();
    assert_eq!(pages(&doc).unwrap()[1].obj, ObjRef::new(5, 0), "the template's own page, not a copy");
    let shown = reopen(&doc);
    assert_eq!(texts(&shown), ["Visible", "Template"]);
    assert_eq!(fields_on(&shown, 1), ["Extra"]);

    // Showing it again changes nothing.
    set_template_hidden(&mut doc, "More", false).unwrap();
    assert_eq!(page_count(&doc).unwrap(), 2);

    set_template_hidden(&mut doc, "More", true).unwrap();
    let page: Dict = doc.get(ObjRef::new(5, 0)).as_dict().cloned().unwrap();
    assert_eq!(page.name(b"Type"), Some(&b"Template"[..]));
    assert!(page.get(b"MediaBox").is_some() && page.get(b"Parent").is_none(), "{page:?}");
    let hidden = reopen(&doc);
    assert_eq!(texts(&hidden), ["Visible"]);
    assert_eq!(templates(&hidden), [Template { name: "More".into(), hidden: true }]);
}

#[test]
fn unknown_templates_and_the_last_page_are_refused() {
    let mut doc = form();
    assert!(spawn_template(&mut doc, "Nope", 0, true).is_err());
    assert!(set_template_hidden(&mut doc, "Nope", false).is_err());
    // A document whose only page is named in /Pages can't hide it.
    let mut only = build(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /Pages << /Names [(Only) 3 0 R] >> >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        "<< /Type /Page /Parent 2 0 R >>",
    ]);
    assert!(matches!(set_template_hidden(&mut only, "Only", true), Err(OrganizeError::WouldRemoveAllPages)));
    assert_eq!(page_count(&only).unwrap(), 1);
}

/// A name tree that lists itself, and one whose entry is not a page, list nothing usable.
#[test]
fn hostile_template_trees_end() {
    let mut doc = build(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /Templates 4 0 R >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Kids [4 0 R 5 0 R] >>",
        "<< /Names [(Broken) 42] >>",
    ]);
    assert_eq!(templates(&doc), []);
    assert!(spawn_template(&mut doc, "Broken", 0, false).is_err());
    assert!(set_template_hidden(&mut doc, "Broken", false).is_err());
}

/// Two copies without renaming: still one field, now with the template's widget and both copies'
/// as its kids, and its value held once, by the field.
#[test]
fn every_unrenamed_copy_is_one_more_widget_of_the_one_field() {
    let mut doc = form();
    spawn_template(&mut doc, "More", 1, false).unwrap();
    spawn_template(&mut doc, "More", 2, false).unwrap();
    let doc = reopen(&doc);
    assert_eq!(listed_fields(&doc), ["Extra"]);
    let catalog = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let form = doc.resolve(catalog.get(b"AcroForm").unwrap()).as_dict().cloned().unwrap();
    let field = doc.resolve(form.get(b"Fields").unwrap()).as_array().cloned().unwrap()[0].as_ref().unwrap();
    let field = doc.get(field).as_dict().cloned().unwrap();
    assert_eq!(field.get(b"V").and_then(|v| v.as_string()).map(|s| s.to_text()).as_deref(), Some("x"), "the value is the field's");
    let kids = doc.resolve(field.get(b"Kids").unwrap()).as_array().cloned().unwrap();
    assert_eq!(kids.len(), 3, "{field:?}");
    for kid in kids.iter().filter_map(Object::as_ref) {
        let w = doc.get(kid).as_dict().cloned().unwrap();
        assert!(w.name(b"Subtype") == Some(&b"Widget"[..]) && !w.contains(b"T") && !w.contains(b"V"), "a plain widget: {w:?}");
    }
    assert_eq!(fields_on(&doc, 1), ["Extra"]);
    assert_eq!(fields_on(&doc, 2), ["Extra"]);
}

/// A visible named page that inherits its media box from the page tree keeps it when hidden:
/// out of the tree, nothing would give it one.
#[test]
fn a_hidden_page_keeps_what_it_inherited() {
    let mut doc = build(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /Pages << /Names [(Cover) 4 0 R] >> >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 210 297] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Page /Parent 2 0 R >>",
    ]);
    set_template_hidden(&mut doc, "Cover", true).unwrap();
    assert_eq!(page_count(&doc).unwrap(), 1);
    let page: Dict = doc.get(ObjRef::new(4, 0)).as_dict().cloned().unwrap();
    let media = page.get(b"MediaBox").and_then(|m| m.as_array().cloned()).expect("its media box, inherited before");
    assert_eq!(media.len(), 4);
    assert_eq!(templates(&doc), [Template { name: "Cover".into(), hidden: true }]);
}
