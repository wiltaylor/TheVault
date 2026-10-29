//! A CSS text colour as the nearest of the nine palette colours (OQ-S4), only when the colour is
//! clearly a colour: black, white, greys and pale or dark tints stay uncoloured, since mail sets
//! a colour on most text.

/// The palette colour nearest `css`, or `None` when it is not clearly a colour.
pub fn nearest(css: &str) -> Option<&'static str> {
    let (r, g, b) = rgb(css.trim())?;
    let (h, s, l) = hsl(r, g, b);
    if s < 0.35 || !(0.2..=0.8).contains(&l) {
        return None;
    }
    Some(match h {
        h if h < 15.0 => "red",
        h if h < 40.0 => {
            if l < 0.4 {
                "brown"
            } else {
                "orange"
            }
        }
        h if h < 70.0 => "yellow",
        h if h < 170.0 => "green",
        h if h < 255.0 => "blue",
        h if h < 290.0 => "purple",
        h if h < 345.0 => "pink",
        _ => "red",
    })
}

fn rgb(css: &str) -> Option<(f64, f64, f64)> {
    let css = css.to_ascii_lowercase();
    if let Some(hex) = css.strip_prefix('#') {
        let n = |s: &str| u8::from_str_radix(s, 16).ok().map(f64::from);
        return match hex.len() {
            3 => {
                let d = |i: usize| n(&hex[i..=i].repeat(2));
                Some((d(0)?, d(1)?, d(2)?))
            }
            6 => Some((n(&hex[0..2])?, n(&hex[2..4])?, n(&hex[4..6])?)),
            _ => None,
        };
    }
    if let Some(args) = css
        .strip_prefix("rgba(")
        .or_else(|| css.strip_prefix("rgb("))
        .and_then(|a| a.strip_suffix(')'))
    {
        let v: Vec<f64> = args
            .split([',', ' ', '/'])
            .filter(|s| !s.is_empty())
            .take(3)
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        return (v.len() == 3).then(|| (v[0], v[1], v[2]));
    }
    Some(match css.as_str() {
        "red" => (255.0, 0.0, 0.0),
        "darkred" => (139.0, 0.0, 0.0),
        "orange" => (255.0, 165.0, 0.0),
        "brown" => (165.0, 42.0, 42.0),
        "yellow" | "gold" => (255.0, 215.0, 0.0),
        "green" => (0.0, 128.0, 0.0),
        "lime" => (0.0, 255.0, 0.0),
        "blue" => (0.0, 0.0, 255.0),
        "navy" => (0.0, 0.0, 128.0),
        "teal" => (0.0, 128.0, 128.0),
        "purple" => (128.0, 0.0, 128.0),
        "violet" => (238.0, 130.0, 238.0),
        "magenta" | "fuchsia" => (255.0, 0.0, 255.0),
        "pink" | "hotpink" => (255.0, 105.0, 180.0),
        _ => return None,
    })
}

/// Hue in degrees, saturation and lightness from 0 to 1.
fn hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s, l)
}
