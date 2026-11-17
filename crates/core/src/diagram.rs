//! Draws FastDraw (`.fdraw`) diagram text to a standalone SVG, for notes that hold a diagram and
//! for Dream to check one before it writes it.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::OnceLock;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fastdraw_render::{Level, Source};

/// Diagram text longer than this many bytes is not drawn.
pub const MAX_BYTES: usize = 1_000_000;

/// The one error reported when drawing panics.
const PANIC_TEXT: &str = "FastDraw could not draw this diagram";

/// A drawn diagram: the SVG (empty when nothing could be drawn) and one `Line n: message` per error.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    pub svg: String,
    pub errors: Vec<String>,
}

/// Draws `text`, which lives in `file` (the base of a relative look path), in the light or dark
/// look. Never panics.
pub fn draw(text: &str, file: Option<&Path>, dark: bool) -> Drawn {
    if text.len() > MAX_BYTES {
        return Drawn {
            svg: String::new(),
            errors: vec!["diagram too large".to_owned()],
        };
    }
    catch_unwind(AssertUnwindSafe(|| draw_unguarded(text, file, dark))).unwrap_or_else(|_| Drawn {
        svg: String::new(),
        errors: vec![PANIC_TEXT.to_owned()],
    })
}

/// The errors in `text`, as `draw` reports them.
pub fn errors(text: &str) -> Vec<String> {
    draw(text, None, false).errors
}

/// `text` without surrounding blanks or one surrounding ``` or ```fdraw fence; `None` when nothing
/// is left.
pub fn unfence(text: &str) -> Option<String> {
    let text = text.trim();
    let inner = match text.strip_prefix("```") {
        Some(rest) => {
            let rest = rest.strip_prefix("fdraw").unwrap_or(rest);
            rest.strip_suffix("```").unwrap_or(rest)
        }
        None => text,
    };
    let inner = inner.trim();
    (!inner.is_empty()).then(|| inner.to_owned())
}

/// Whether a look path is one FastMind will not read: absolute, a network path, or with a drive
/// prefix (`C:x.look` included).
fn refused(look: &str) -> bool {
    look.starts_with(['/', '\u{5c}']) || look.get(1..2) == Some(":")
}

fn refusal(look: &str) -> String {
    format!("FastMind reads look files only by a relative path; \"{look}\" was not read")
}

fn draw_unguarded(text: &str, file: Option<&Path>, dark: bool) -> Drawn {
    let refused_look = fastdraw_render::look_path(text).filter(|look| refused(look));
    // A refused look is never handed to the renderer's file reading.
    let file = if refused_look.is_some() { None } else { file };
    let drawing = fastdraw_render::render(&Source { text, file, dark });

    let errors = drawing
        .issues
        .iter()
        .filter(|i| i.level == Level::Error)
        .map(|issue| {
            let message = match &refused_look {
                Some(look) if issue.message.starts_with("look file not found") => refusal(look),
                _ => issue.message.clone(),
            };
            format!("Line {}: {message}", issue.line)
        })
        .collect();
    Drawn {
        svg: embed_fonts(&drawing.svg, &drawing.font_files),
        errors,
    }
}

/// One bundled or look-supplied font, ready to embed.
struct Face {
    family: String,
    weight: u32,
    italic: bool,
    base64: String,
}

fn face(bytes: &[u8]) -> Option<Face> {
    // `fontmeta` is not in the crate's documented API; the crate is pinned to a commit, so this is safe.
    let meta = fastdraw_render::fontmeta::font_meta(bytes)?;
    Some(Face {
        family: meta.family,
        weight: meta.weight,
        italic: meta.italic,
        base64: STANDARD.encode(bytes),
    })
}

/// The bundled fonts, encoded once.
fn bundled_faces() -> &'static [Face] {
    static FACES: OnceLock<Vec<Face>> = OnceLock::new();
    FACES.get_or_init(|| {
        fastdraw_render::fonts()
            .iter()
            .filter_map(|(_, bytes)| face(bytes))
            .collect()
    })
}

/// The font family names the SVG's `font-family="..."` attributes use.
fn families_used(svg: &str) -> BTreeSet<String> {
    svg.split("font-family=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .flat_map(|list| list.split(','))
        .map(|name| name.trim().trim_matches(['\'', '"']).to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// `svg` with an `@font-face` for each bundled or look-supplied font its text names, so it draws
/// the same wherever it is shown.
fn embed_fonts(svg: &str, font_files: &[std::path::PathBuf]) -> String {
    let used = families_used(svg);
    let own: Vec<Face> = font_files
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .filter_map(|bytes| face(&bytes))
        .collect();
    let mut css = String::new();
    for f in bundled_faces().iter().chain(&own) {
        if used.contains(&f.family) {
            css.push_str(&format!(
                "@font-face{{font-family:\"{}\";font-weight:{};font-style:{};src:url(data:font/ttf;base64,{})}}",
                f.family,
                f.weight,
                if f.italic { "italic" } else { "normal" },
                f.base64
            ));
        }
    }
    if css.is_empty() {
        return svg.to_owned();
    }
    match svg.find('>') {
        Some(end) if svg.starts_with("<svg") => {
            format!("{}<style>{css}</style>{}", &svg[..=end], &svg[end + 1..])
        }
        _ => svg.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "a: focal \"A\" at 40,40\nb: store \"B\" at 300,40\na -> b \"x\"";

    #[test]
    fn draws_a_valid_diagram() {
        let d = draw(GOOD, None, false);
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert!(d.svg.starts_with("<svg"));
        assert!(d.svg.contains("@font-face"));
        let used = families_used(&d.svg);
        assert!(!used.is_empty());
        for family in used
            .iter()
            .filter(|u| bundled_faces().iter().any(|f| &f.family == *u))
        {
            let rule = format!("@font-face{{font-family:\"{family}\"");
            assert!(d.svg.contains(&rule), "no {rule}");
        }
        assert!(
            used.iter()
                .any(|u| bundled_faces().iter().any(|f| &f.family == u)),
            "no bundled family in {used:?}"
        );
    }

    #[test]
    fn panic_text_is_the_specified_one() {
        assert_eq!(PANIC_TEXT, "FastDraw could not draw this diagram");
    }

    #[test]
    fn unknown_shape_is_a_line_error() {
        let d = draw(
            "a: focal \"A\" at 40,40\nb: nosuchshape \"B\" at 300,40",
            None,
            false,
        );
        assert!(d.errors[0].starts_with("Line 2: "), "{:?}", d.errors);
    }

    #[test]
    fn refuses_absolute_look_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("d.fdraw");
        for p in [
            "//server/x.look",
            "\\\\server\\x.look",
            "C:/x.look",
            "C:x.look",
            "/x.look",
        ] {
            // FastDraw strings escape a backslash with another.
            let text = format!("look \"{}\"\na: rect at 40,40", p.replace('\\', "\\\\"));
            let d = draw(&text, Some(&file), false);
            let want = format!(
                "Line 1: FastMind reads look files only by a relative path; \"{p}\" was not read"
            );
            assert_eq!(d.errors, [want]);
            assert!(!d.svg.is_empty());
        }
    }

    #[test]
    fn relative_and_bundled_looks_pass() {
        let name = fastdraw_render::look::bundled_look_names()
            .into_iter()
            .find(|n| *n == "night")
            .or_else(|| {
                fastdraw_render::look::bundled_look_names()
                    .into_iter()
                    .next()
            })
            .unwrap();
        let d = draw(
            &format!("look \"{name}\"\na: focal \"A\" at 40,40"),
            None,
            false,
        );
        assert!(d.errors.is_empty(), "{:?}", d.errors);

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.look"), "").unwrap();
        let file = dir.path().join("d.fdraw");
        let d = draw(
            "look \"x.look\"\na: focal \"A\" at 40,40",
            Some(&file),
            false,
        );
        assert!(
            !d.errors.iter().any(|e| e.contains("FastMind reads")),
            "{:?}",
            d.errors
        );
    }

    #[test]
    fn faces_carry_their_weight_and_style() {
        let bold = bundled_faces()
            .iter()
            .find(|f| f.weight >= 600)
            .expect("a bold bundled face");
        let svg = format!("<svg><text font-family=\"{}\">x</text></svg>", bold.family);
        let out = embed_fonts(&svg, &[]);
        assert!(
            out.contains(&format!("font-weight:{};", bold.weight)),
            "{}",
            &out[..200.min(out.len())]
        );
        assert!(out.contains("font-style:"));
    }

    #[test]
    fn too_large_is_refused() {
        let d = draw(&"a".repeat(MAX_BYTES + 1), None, false);
        assert_eq!(d.errors, ["diagram too large"]);
    }

    #[test]
    fn dark_draw_differs() {
        assert_ne!(draw(GOOD, None, true).svg, draw(GOOD, None, false).svg);
    }

    #[test]
    fn diagram_text_is_unfenced_and_blank_is_none() {
        assert_eq!(unfence("  "), None);
        assert_eq!(
            unfence("```fdraw\na: focal \"A\" at 0,0\n```").as_deref(),
            Some("a: focal \"A\" at 0,0")
        );
        assert_eq!(errors(GOOD), Vec::<String>::new());
    }
}
