//! The few inline CSS properties the converter reads from a `style` attribute: whether an element
//! shows, its text marks and colour, a button's look, an image's size and a cell's alignment.

/// The declarations of a `style` attribute, names lower case, values trimmed, `!important`
/// dropped.
pub fn declarations(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|d| {
            let (name, value) = d.split_once(':')?;
            let value = value.trim();
            let value = value
                .strip_suffix("!important")
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase();
            Some((name.trim().to_ascii_lowercase(), value))
        })
        .collect()
}

pub fn get<'a>(decls: &'a [(String, String)], name: &str) -> Option<&'a str> {
    decls
        .iter()
        .rev()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// Whether the declarations hide what they style: `display:none`, `visibility:hidden`,
/// `mso-hide:all`, `opacity:0`, or a zero `max-height` or `font-size` with the overflow hidden,
/// as a preheader hides itself.
pub fn hides(decls: &[(String, String)]) -> bool {
    let zero = |v: Option<&str>| v.is_some_and(|v| px(v) == Some(0.0));
    get(decls, "display") == Some("none")
        || get(decls, "visibility") == Some("hidden")
        || get(decls, "mso-hide") == Some("all")
        || get(decls, "opacity").is_some_and(|v| v.parse::<f64>().ok() == Some(0.0))
        || ((zero(get(decls, "max-height")) || zero(get(decls, "height")))
            && get(decls, "overflow") == Some("hidden"))
        || (zero(get(decls, "font-size")) && zero(get(decls, "line-height")))
}

/// A length in CSS pixels: `12`, `12px` or `9pt`. `None` for a percentage, `auto` or anything
/// else.
pub fn px(v: &str) -> Option<f64> {
    let v = v.trim();
    if let Some(n) = v.strip_suffix("px") {
        return n.trim().parse().ok();
    }
    if let Some(n) = v.strip_suffix("pt") {
        return n.trim().parse::<f64>().ok().map(|p| p * 4.0 / 3.0);
    }
    v.parse().ok()
}

/// An HTML `width` or `height` attribute in pixels: `1`, `1px`; `None` for `100%` or nothing.
pub fn attr_px(v: Option<&str>) -> Option<f64> {
    let v = v?.trim();
    if v.ends_with('%') {
        return None;
    }
    let digits: String = v
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse().ok()
}

/// Whether `font-weight` is bold.
pub fn bold(decls: &[(String, String)]) -> Option<bool> {
    let v = get(decls, "font-weight")?;
    Some(match v {
        "bold" | "bolder" => true,
        "normal" | "lighter" => false,
        n => n.parse::<u32>().is_ok_and(|w| w >= 600),
    })
}

pub fn italic(decls: &[(String, String)]) -> Option<bool> {
    let v = get(decls, "font-style")?;
    Some(v == "italic" || v == "oblique")
}

/// The `text-decoration` (or `text-decoration-line`) words: underline and line-through.
pub fn decoration(decls: &[(String, String)]) -> (bool, bool) {
    let v = get(decls, "text-decoration-line")
        .or_else(|| get(decls, "text-decoration"))
        .unwrap_or("");
    (v.contains("underline"), v.contains("line-through"))
}

/// The alignment of a table cell: `left`, `center` or `right`.
pub fn align(attr: Option<&str>, decls: &[(String, String)]) -> Option<&'static str> {
    let v = get(decls, "text-align")
        .map(str::to_owned)
        .or_else(|| attr.map(|a| a.trim().to_ascii_lowercase()))?;
    match v.as_str() {
        "center" | "middle" => Some("center"),
        "right" | "end" => Some("right"),
        "left" | "start" | "justify" => Some("left"),
        _ => None,
    }
}

/// Whether the declarations make a link look like a button: a background, or a border with
/// padding.
pub fn button(decls: &[(String, String)]) -> bool {
    let background = get(decls, "background-color")
        .or_else(|| get(decls, "background"))
        .is_some_and(|v| !matches!(v, "none" | "transparent" | "inherit" | "initial"));
    let border = get(decls, "border").is_some_and(|v| v != "none" && v != "0");
    let padding = decls.iter().any(|(n, _)| n.starts_with("padding"));
    background || (border && padding)
}
