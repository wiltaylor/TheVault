//! Small crafted PDFs that make pdf-extract 0.12 panic, built in code so no binary fixture is
//! needed. Each reproduces one panic seen on real mail attachments.

/// A PDF of `objects`, numbered from 1, the first being the catalog, with a correct xref table.
fn build(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(content: &str) -> String {
    format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

/// One page, its font as object 4, its content as object 5, and `extra` from object 6 on.
fn one_page(resources: &str, font: &str, content: &str, extra: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 4 0 R >> {resources} >> /Contents 5 0 R >>"
        ),
        font.to_owned(),
        stream(content),
    ];
    objects.extend_from_slice(extra);
    build(&objects)
}

/// A page that fills with a Separation colour space whose Lab alternate names its dictionary
/// by reference, which pdf-extract wants inline: "second arg must be a dict". The text on it,
/// "quokka marmalade", is plain Helvetica.
pub fn colour_space_by_reference() -> Vec<u8> {
    one_page(
        "/ColorSpace << /CS0 [/Separation /Spot [/Lab 6 0 R] 7 0 R] >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "/CS0 cs 1 scn BT /F1 12 Tf 72 720 Td (quokka marmalade) Tj ET",
        &[
            "<< /WhitePoint [0.9505 1 1.089] /Range [-100 100 -100 100] >>".to_owned(),
            "<< /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 0] /N 1 >>".to_owned(),
        ],
    )
}

const CMAP_A_TO_Z: &str = "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
/CMapName /AtoZ def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
1 beginbfrange\n<41> <5A> <0041>\nendbfrange\nendcmap\n\
CMapName currentdict /CMap defineresource pop\nend\nend";

/// A font with a ToUnicode map that covers A to Z only and no encoding, showing `shown` (a PDF
/// string body) in it. A code outside the map makes pdf-extract panic: "missing unicode map
/// and encoding".
fn unmapped_font(shown: &str) -> Vec<u8> {
    one_page(
        "",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Quill /ToUnicode 6 0 R >>",
        &format!("BT /F1 12 Tf 72 720 Td ({shown}) Tj ET"),
        &[stream(CMAP_A_TO_Z)],
    )
}

/// [`unmapped_font`] showing "WOMBAT" then a code the map lacks. A tolerant extractor reads
/// the word.
pub fn font_without_encoding() -> Vec<u8> {
    unmapped_font("WOMBAT \\001")
}

/// [`unmapped_font`] showing only codes the map lacks, so no extractor finds any text.
pub fn font_without_encoding_or_text() -> Vec<u8> {
    unmapped_font("\\001\\002\\003")
}
