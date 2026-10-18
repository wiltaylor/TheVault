//! PDFs that make pdf-extract panic: the panic is caught, a tolerant fallback reads what text
//! it can, and a PDF neither can read fails with the panic's message as its reason.

#[path = "support/hostile_pdf.rs"]
mod hostile_pdf;

use index::{Extracted, extract_pdf_text};

/// The message pdf-extract panics with on `bytes`, proving the crafted PDF reproduces it.
fn pdf_extract_panic(bytes: &[u8]) -> String {
    let out = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes));
    let payload = out.expect_err("pdf-extract read the PDF without panicking");
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

#[test]
fn a_colour_space_by_reference_falls_back_to_a_tolerant_extractor() {
    let pdf = hostile_pdf::colour_space_by_reference();
    assert!(pdf_extract_panic(&pdf).contains("second arg must be a dict"));
    let Extracted::Recovered { text, reason } = extract_pdf_text(pdf) else {
        panic!("not recovered");
    };
    assert!(
        reason.contains("panicked") && !reason.contains('\n'),
        "{reason}"
    );
    assert!(text.contains("quokka marmalade"), "{text:?}");
}

#[test]
fn a_font_with_no_encoding_falls_back_to_a_tolerant_extractor() {
    let pdf = hostile_pdf::font_without_encoding();
    assert!(pdf_extract_panic(&pdf).contains("missing unicode map and encoding"));
    let Extracted::Recovered { text, reason } = extract_pdf_text(pdf) else {
        panic!("not recovered");
    };
    assert!(
        reason.contains("panicked") && !reason.contains('\n'),
        "{reason}"
    );
    assert!(text.contains("WOMBAT"), "{text:?}");
    assert!(!text.contains('\u{FFFD}'), "{text:?}");
}

#[test]
fn a_pdf_neither_extractor_reads_fails_with_the_panic_as_its_reason() {
    let pdf = hostile_pdf::font_without_encoding_or_text();
    assert!(pdf_extract_panic(&pdf).contains("missing unicode map and encoding"));
    match extract_pdf_text(pdf) {
        Extracted::Failed(reason) => {
            assert!(
                reason.contains("missing unicode map and encoding"),
                "{reason}"
            );
            assert!(!reason.contains('\n'), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}
