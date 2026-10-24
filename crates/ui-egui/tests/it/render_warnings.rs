//! A page the renderer draws only in part or approximately is marked as such in the viewer, not
//! shown as if it were complete (#4340).

use egui_kittest::Harness;
use pdfcraft_render::RenderWarning;
use pdfcraft_ui_egui::PdfCraftApp;

/// One 300 × 400 pt page painted in a DeviceN colour whose tint transform `program` (a
/// calculator function taking the two tints) gives the DeviceRGB colour.
fn devicen_pdf(program: &str) -> Vec<u8> {
    let content = "/CS1 cs 0.5 0.5 scn 0 0 150 200 re f";
    format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /ColorSpace << /CS1 [/DeviceN [/Spot1 /Spot2] /DeviceRGB 5 0 R] >> >> /Contents 4 0 R >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n\
         5 0 obj << /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length {} >> stream\n{program}\nendstream endobj\n\
         trailer << /Root 1 0 R >>\n%%EOF",
        content.len(),
        program.len()
    )
    .into_bytes()
}

/// The app with `bytes` open, run until every requested raster has arrived.
fn rendered(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("warned.pdf", None, bytes).expect("opens");
        app.set_option("left", "closed").unwrap();
        app.set_option("panel", "none").unwrap();
        app
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        assert!(std::time::Instant::now() < deadline, "render readiness timed out: {:?}", h.state().views[0].page_errors());
        h.run_steps(2);
        if !h.state().render_pending() {
            h.run_steps(1);
            if !h.state().render_pending() {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(h.state().views[0].page_errors().is_empty(), "{:?}", h.state().views[0].page_errors());
    h
}

#[test]
fn a_page_with_a_fallback_colour_is_marked_in_the_viewer() {
    // The tint transform pops more than it is given, so the colour can't be computed.
    let h = rendered(devicen_pdf("{ pop pop pop }"));
    assert_eq!(h.state().views[0].page_warnings(), vec![(0, &[RenderWarning::ColourFallback][..])]);
}

#[test]
fn a_page_drawn_as_specified_is_not_marked() {
    let h = rendered(devicen_pdf("{ pop pop 0 0 0 }"));
    assert!(h.state().views[0].page_warnings().is_empty(), "{:?}", h.state().views[0].page_warnings());
}

#[test]
fn the_warning_note_closes_with_its_button() {
    let mut h = rendered(devicen_pdf("{ pop pop pop }"));
    h.run_steps(2);
    assert!(h.state().views[0].render_warning_note(0).is_some());
    h.get_by_label("Dismiss").click();
    h.run_steps(2);
    assert_eq!(h.state().views[0].render_warning_note(0), None, "the note is closed");
    assert_eq!(h.state().views[0].page_warnings(), vec![(0, &[RenderWarning::ColourFallback][..])], "the page is still reported");
}
