//! The image in a Windows notification's logo slot: a pilot's tag on the
//! overlays' steel-glass plate, with the pilot's accent on the left edge and
//! a notch in the reason's color along the top (docs/design/
//! windows-notification.html). A notification can't be styled with CSS, so
//! this image is where the overlays' look carries over.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Mask, Paint, PathBuilder, Pixmap, Point, Rect, SpreadMode, Stroke, Transform};

/// Rendered at twice the 48px slot, so it stays sharp on high-DPI screens.
pub const SIZE: u32 = 96;
const RADIUS: f32 = 10.0;

/// Mirrors the overlays' reason colors (`--mention`, `--keyword`, `--always`
/// in overlay.css).
pub fn tone_color(tone: &str) -> Color {
    match tone {
        "mention" => rgb(0xe3, 0xa5, 0x3a),
        "keyword" => rgb(0x55, 0xc4, 0xd6),
        _ => rgb(0x6f, 0x9f, 0xe0),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// "#62d1a5" (as `runner::accent_for` returns) to a color; anything else
/// falls back to the overlays' ink color rather than failing the alert.
pub fn parse_hex(s: &str) -> Color {
    let h = s.trim_start_matches('#');
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    match (h.len(), byte(0), byte(2), byte(4)) {
        (6, Some(r), Some(g), Some(b)) => rgb(r, g, b),
        _ => rgb(0xdb, 0xe5, 0xea),
    }
}

fn rounded(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let (l, t, rt, b) = (r.left(), r.top(), r.right(), r.bottom());
    let mut p = PathBuilder::new();
    p.move_to(l + radius, t);
    p.line_to(rt - radius, t);
    p.quad_to(rt, t, rt, t + radius);
    p.line_to(rt, b - radius);
    p.quad_to(rt, b, rt - radius, b);
    p.line_to(l + radius, b);
    p.quad_to(l, b, l, b - radius);
    p.line_to(l, t + radius);
    p.quad_to(l, t, l + radius, t);
    p.close();
    p.finish()
}

/// Renders the badge as PNG bytes. `font` is the tag's typeface; without
/// one the plate is drawn without text rather than failing.
pub fn render(tag: &str, accent: Color, tone: Color, font: Option<&FontVec>) -> Option<Vec<u8>> {
    let s = SIZE as f32;
    let mut pm = Pixmap::new(SIZE, SIZE)?;
    let plate = rounded(Rect::from_xywh(0.0, 0.0, s, s)?, RADIUS)?;

    // Steel glass, as `.glass` in overlay.css.
    let glass = Paint {
        anti_alias: true,
        shader: LinearGradient::new(
            Point::from_xy(s * 0.35, 0.0),
            Point::from_xy(s * 0.65, s),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(70, 96, 110, 245)),
                GradientStop::new(1.0, Color::from_rgba8(24, 43, 51, 250)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )?,
        ..Paint::default()
    };
    pm.fill_path(&plate, &glass, FillRule::Winding, Transform::identity(), None);

    // Accent edge on the left, and the reason notch on top, both kept
    // inside the plate's rounded corners.
    let mut inside = Mask::new(SIZE, SIZE)?;
    inside.fill_path(&plate, FillRule::Winding, true, Transform::identity());
    let solid = |c: Color| {
        let mut p = Paint::default();
        p.set_color(c);
        p.anti_alias = true;
        p
    };
    let edge = PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, 6.0, s)?);
    pm.fill_path(&edge, &solid(accent), FillRule::Winding, Transform::identity(), Some(&inside));
    let mut glow = tone;
    glow.set_alpha(0.35);
    let halo = PathBuilder::from_rect(Rect::from_xywh(s / 2.0 - 28.0, 0.0, 56.0, 9.0)?);
    pm.fill_path(&halo, &solid(glow), FillRule::Winding, Transform::identity(), Some(&inside));
    let notch = PathBuilder::from_rect(Rect::from_xywh(s / 2.0 - 22.0, 0.0, 44.0, 5.0)?);
    pm.fill_path(&notch, &solid(tone), FillRule::Winding, Transform::identity(), Some(&inside));

    // Hairline edge, as `--edge`.
    let mut line = solid(Color::from_rgba8(190, 215, 228, 60));
    line.anti_alias = true;
    pm.stroke_path(&plate, &line, &Stroke { width: 2.0, ..Stroke::default() }, Transform::identity(), None);

    if let Some(font) = font {
        draw_tag(&mut pm, tag, accent, font);
    }
    pm.encode_png().ok()
}

/// Centers the tag, shrinking it so up to five characters fit.
fn draw_tag(pm: &mut Pixmap, tag: &str, color: Color, font: &FontVec) {
    let s = SIZE as f32;
    let max_w = s - 22.0;
    let width_at = |px: f32| {
        let sf = font.as_scaled(PxScale::from(px));
        tag.chars().map(|c| sf.h_advance(font.glyph_id(c))).sum::<f32>()
    };
    let mut px = 40.0;
    let w = width_at(px);
    if w > max_w {
        px *= max_w / w;
    }
    let scale = PxScale::from(px);
    let sf = font.as_scaled(scale);
    let total = width_at(px);
    let mut x = (s - total) / 2.0 + 2.0; // a nudge right, off the accent edge
    let baseline = (s + sf.ascent() + sf.descent()) / 2.0 + 2.0;
    let (cr, cg, cb) = (color.red(), color.green(), color.blue());
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    for ch in tag.chars() {
        let id = font.glyph_id(ch);
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, baseline));
        x += sf.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let b = outline.px_bounds();
        let data = pm.data_mut();
        outline.draw(|gx, gy, cov| {
            let (px_, py_) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
            if px_ < 0 || py_ < 0 || px_ >= w || py_ >= h || cov <= 0.0 {
                return;
            }
            // Source-over onto premultiplied RGBA.
            let i = ((py_ * w + px_) * 4) as usize;
            let a = cov.min(1.0);
            let blend = |dst: u8, src: f32| (src * 255.0 * a + dst as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            data[i] = blend(data[i], cr);
            data[i + 1] = blend(data[i + 1], cg);
            data[i + 2] = blend(data[i + 2], cb);
            data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_parse_and_bad_ones_fall_back() {
        let c = parse_hex("#62d1a5");
        assert_eq!(((c.red() * 255.0).round(), (c.green() * 255.0).round(), (c.blue() * 255.0).round()), (98.0, 209.0, 165.0));
        let fallback = parse_hex("nope");
        assert!((fallback.red() - 0xdb as f32 / 255.0).abs() < 1e-6);
    }

    /// Writes sample badges with the real font to `$BADGE_SAMPLES`, for
    /// eyeballing: `BADGE_SAMPLES=<dir> cargo test -p eve-chatterer-app
    /// badge_samples -- --ignored`.
    #[test]
    #[ignore]
    fn badge_samples() {
        let dir = std::path::PathBuf::from(std::env::var("BADGE_SAMPLES").expect("set BADGE_SAMPLES"));
        let font = std::fs::read(r"C:\Windows\Fonts\seguisb.ttf").ok().and_then(|b| FontVec::try_from_vec(b).ok());
        for (tag, accent, tone) in [("J", "#62d1a5", "mention"), ("PA", "#a58af0", "keyword"), ("CA", "#6cb6f0", "always"), ("NIGHT", "#e58aa8", "keyword")] {
            let png = render(tag, parse_hex(accent), tone_color(tone), font.as_ref()).unwrap();
            std::fs::write(dir.join(format!("badge-{tag}.png")), png).unwrap();
        }
    }

    #[test]
    fn renders_a_png_of_the_right_size_with_or_without_a_font() {
        let png = render("PA", parse_hex("#a58af0"), tone_color("keyword"), None).expect("png");
        assert_eq!(&png[1..4], b"PNG");
        let back = Pixmap::decode_png(&png).unwrap();
        assert_eq!((back.width(), back.height()), (SIZE, SIZE));
    }
}
