//! The window's chrome in light and dark: text sizes legible on a 1080p
//! screen and on Retina, and the colours the chrome draws text and marks
//! with, per theme (the die's map keeps its own dark canvas in both).
use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};

/// Colours of text and marks on the chrome: an error, quieter text, a net's
/// name, an LED at 0 or unknown.
pub struct Colours {
    pub error: Color32,
    pub muted: Color32,
    pub net: Color32,
    /// Tour guidance uses green, outside the map's wire palette.
    pub tour: Color32,
    /// Critical timing uses pink, separate from wires and tour guidance.
    pub critical: Color32,
    pub critical_text: Color32,
    pub zero: Color32,
    pub unknown: Color32,
}

/// The chrome's colours on a dark or a light theme.
pub fn colours(dark: bool) -> Colours {
    if dark {
        Colours {
            error: Color32::from_rgb(255, 150, 150),
            muted: Color32::from_gray(135),
            net: Color32::from_rgb(100, 225, 250),
            tour: Color32::from_rgb(75, 230, 125),
            critical: Color32::from_rgb(255, 57, 127),
            critical_text: Color32::BLACK,
            zero: Color32::from_rgb(219, 186, 142),
            unknown: Color32::from_rgb(205, 180, 240),
        }
    } else {
        Colours {
            error: Color32::from_rgb(170, 28, 35),
            muted: Color32::from_gray(80),
            net: Color32::from_rgb(0, 92, 111),
            tour: Color32::from_rgb(25, 122, 64),
            critical: Color32::from_rgb(213, 30, 100),
            critical_text: Color32::WHITE,
            zero: Color32::from_rgb(112, 71, 18),
            unknown: Color32::from_rgb(99, 66, 141),
        }
    }
}

/// Source colours share the editor's font metrics in both themes.
pub struct SyntaxColours {
    pub keyword: Color32,
    pub number: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub directive: Color32,
    pub system_task: Color32,
}

pub fn syntax_colours(dark: bool) -> SyntaxColours {
    if dark {
        SyntaxColours {
            keyword: Color32::from_rgb(190, 170, 255),
            number: Color32::from_rgb(240, 195, 140),
            string: Color32::from_rgb(160, 215, 150),
            comment: Color32::from_rgb(155, 170, 155),
            directive: Color32::from_rgb(245, 165, 200),
            system_task: Color32::from_rgb(120, 210, 235),
        }
    } else {
        SyntaxColours {
            keyword: Color32::from_rgb(85, 40, 155),
            number: Color32::from_rgb(125, 65, 10),
            string: Color32::from_rgb(30, 100, 35),
            comment: Color32::from_rgb(70, 95, 70),
            directive: Color32::from_rgb(145, 35, 85),
            system_task: Color32::from_rgb(0, 90, 115),
        }
    }
}

/// egui's own fonts, with its monospace font (Hack) also the proportional
/// font's last fallback: the proportional font has no arrows (→ ← ⇒ ▾), and a
/// character no font has is drawn as a box.
fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    if fonts.font_data.contains_key("Hack") {
        if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
            if !proportional.iter().any(|f| f == "Hack") {
                proportional.push("Hack".to_owned());
            }
        }
    }
    fonts
}

/// Whether `font` draws `c` with a glyph of its own, not the box a missing
/// character gets. (egui's `has_glyph` also says no for every character of
/// the face that holds the box, such as all the monospace font's letters.)
#[cfg(test)]
pub(crate) fn draws(ctx: &egui::Context, font: &FontId, c: char) -> bool {
    let cell = |c: char| {
        ctx.fonts_mut(|f| {
            let galley = f.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE);
            galley
                .rows
                .first()
                .and_then(|r| r.glyphs.first())
                .map(|g| g.uv_rect)
        })
    };
    c.is_whitespace() || cell(c) != cell('\u{25FB}')
}

/// The theme chosen (System follows the OS), and both themes' text sizes
/// and quieter text.
pub fn install(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.set_theme(preference);
    ctx.set_fonts(fonts());
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            for (text, size, family) in [
                (TextStyle::Small, 11.0, FontFamily::Proportional),
                (TextStyle::Body, 13.5, FontFamily::Proportional),
                (TextStyle::Button, 13.5, FontFamily::Proportional),
                (TextStyle::Monospace, 12.5, FontFamily::Monospace),
                (TextStyle::Heading, 18.0, FontFamily::Proportional),
            ] {
                style.text_styles.insert(text, FontId::new(size, family));
            }
            let palette = colours(theme == egui::Theme::Dark);
            style.visuals.weak_text_color = Some(palette.muted);
            style.visuals.error_fg_color = palette.error;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f64 {
        let linear = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
    }

    fn over(fg: Color32, bg: Color32) -> Color32 {
        let inv = 1.0 - fg.a() as f64 / 255.0;
        let ch = |f: u8, b: u8| (f as f64 + b as f64 * inv).round().clamp(0.0, 255.0) as u8;
        Color32::from_rgb(ch(fg.r(), bg.r()), ch(fg.g(), bg.g()), ch(fg.b(), bg.b()))
    }

    #[test]
    fn chrome_text_meets_wcag_contrast_in_both_themes() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let p = colours(theme == egui::Theme::Dark);
            let visuals = theme.default_visuals();
            for (name, colour) in [
                ("error", p.error),
                ("muted", p.muted),
                ("net", p.net),
                ("zero", p.zero),
                ("unknown", p.unknown),
                ("body", visuals.text_color()),
            ] {
                for background in [
                    visuals.panel_fill,
                    visuals.window_fill,
                    visuals.extreme_bg_color,
                    over(visuals.faint_bg_color, visuals.panel_fill),
                ] {
                    let (a, b) = (luminance(colour), luminance(background));
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 4.5, "{theme:?} {name}: {contrast:.2}");
                }
            }
        }
    }

    /// The private map constants stay private; read their declarations so
    /// palette edits there also exercise this test without changing that file.
    fn fabric_colour(name: &str) -> Color32 {
        let source = include_str!("fabric_view.rs");
        let prefix = format!("const {name}: Color32 = Color32::from_rgb(");
        let components: Vec<u8> = source
            .split_once(&prefix)
            .unwrap()
            .1
            .split_once(')')
            .unwrap()
            .0
            .split(',')
            .map(|v| v.trim().parse().unwrap())
            .collect();
        Color32::from_rgb(components[0], components[1], components[2])
    }

    #[test]
    fn critical_and_tour_colours_are_separate_from_wires_and_read_in_both_themes() {
        use crate::fabric_view::{CHANGE, NET, ONE, UNKNOWN, USED, ZERO};
        let wire_colours = [
            USED,
            CHANGE,
            ONE,
            ZERO,
            UNKNOWN,
            NET,
            Color32::WHITE,
            Color32::from_gray(60),
        ];
        let distance = |a: Color32, b: Color32| {
            let delta = |a: u8, b: u8| (f64::from(a) - f64::from(b)).powi(2);
            (delta(a.r(), b.r()) + delta(a.g(), b.g()) + delta(a.b(), b.b())).sqrt()
        };
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let palette = colours(theme == egui::Theme::Dark);
            let all: Vec<_> = [palette.critical, palette.tour]
                .into_iter()
                .chain(wire_colours)
                .collect();
            // Even the two closest existing wire shades (USED and ONE) stay
            // 32 RGB units apart. Kept and hovered nets share the NET colour.
            for (i, &a) in all.iter().enumerate() {
                for &b in &all[..i] {
                    assert!(distance(a, b) >= 32.0, "{theme:?}: {a:?} and {b:?}");
                }
            }
            for (mark, margin) in [(palette.critical, 80.0), (palette.tour, 64.0)] {
                for wire in wire_colours {
                    // Include every orange stripe/glow blend, not just its ends.
                    for glow in 0..=255 {
                        let wire = wire.lerp_to_gamma(CHANGE, glow as f32 / 255.0);
                        assert!(
                            distance(mark, wire) >= margin,
                            "{theme:?}: {mark:?} and {wire:?}"
                        );
                    }
                }
            }
            let visuals = theme.default_visuals();
            for background in [
                fabric_colour("BACKGROUND"),
                visuals.panel_fill,
                visuals.window_fill,
            ] {
                for mark in [palette.tour, palette.critical] {
                    let (a, b) = (luminance(mark), luminance(background));
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 3.0, "{theme:?} {background:?}: {contrast:.2}");
                }
            }
            let (a, b) = (
                luminance(palette.critical),
                luminance(palette.critical_text),
            );
            assert!(
                (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5,
                "badge text contrast"
            );
        }
    }

    #[test]
    fn syntax_colours_read_on_the_editor_and_cross_probe_backgrounds() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let p = syntax_colours(theme == egui::Theme::Dark);
            let visuals = theme.default_visuals();
            for colour in [
                p.keyword,
                p.number,
                p.string,
                p.comment,
                p.directive,
                p.system_task,
            ] {
                for background in [
                    visuals.panel_fill,
                    visuals.extreme_bg_color,
                    over(
                        Color32::from_rgba_unmultiplied(180, 130, 40, 65),
                        visuals.panel_fill,
                    ),
                ] {
                    let (a, b) = (luminance(colour), luminance(background));
                    let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                    assert!(contrast >= 4.5, "{theme:?} {colour:?}: {contrast:.2}");
                }
            }
        }
    }

    /// The characters outside ASCII in the window's own strings (string
    /// literals in gui/src, comments left out), with where each first is.
    fn characters_in_strings() -> Vec<(char, String)> {
        fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, files);
                } else if path.extension().is_some_and(|x| x == "rs") {
                    files.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut found: Vec<(char, String)> = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                // Code before a line comment; inside it, the quoted parts.
                let code = line.split("//").next().unwrap_or("");
                let mut inside = false;
                let mut escaped = false;
                for c in code.chars() {
                    match (inside, escaped, c) {
                        (true, false, '\\') => escaped = true,
                        (_, false, '"') => inside = !inside,
                        (true, _, c) if !c.is_ascii() => {
                            if !found.iter().any(|f| f.0 == c) {
                                let at = format!(
                                    "{}:{}",
                                    file.file_name().unwrap().to_string_lossy(),
                                    n + 1
                                );
                                found.push((c, at));
                            }
                            escaped = false;
                        }
                        _ => escaped = false,
                    }
                }
            }
        }
        found
    }

    /// Every character the window's strings use is drawn, not as a box, in
    /// the proportional and the monospace font (a string may be shown in
    /// either).
    #[test]
    fn the_windows_strings_have_every_glyph() {
        let ctx = egui::Context::default();
        install(&ctx, egui::ThemePreference::Dark);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let missing: Vec<String> = characters_in_strings()
            .into_iter()
            .flat_map(|(c, at)| {
                [FontId::proportional(13.0), FontId::monospace(13.0)]
                    .into_iter()
                    .filter(|font| !draws(&ctx, font, c))
                    .map(|font| format!("{c:?} (U+{:04X}) at {at}: {:?}", c as u32, font.family))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(missing.is_empty(), "no glyph for {missing:#?}");
    }

    /// The fonts' arrows and marks the window uses: the check above would
    /// pass on an empty set of strings.
    #[test]
    fn arrows_are_drawn_and_a_missing_glyph_is_caught() {
        let ctx = egui::Context::default();
        install(&ctx, egui::ThemePreference::Dark);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        for font in [FontId::proportional(13.0), FontId::monospace(13.0)] {
            for c in "a→←⇒↔▾▶◀✔✖×·…–—−".chars() {
                assert!(draws(&ctx, &font, c), "{c:?} in {:?}", font.family);
            }
            // In no font: drawn as a box.
            assert!(!draws(&ctx, &font, '✓'), "{:?}", font.family);
        }
        assert!(!characters_in_strings().is_empty());
    }

    #[test]
    fn text_styles_are_at_least_eleven_points() {
        let ctx = egui::Context::default();
        install(&ctx, egui::ThemePreference::System);
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let style = ctx.style_of(theme);
            for (text, font) in &style.text_styles {
                assert!(font.size >= 11.0, "{theme:?} {text:?}: {}", font.size);
            }
        }
    }
}
