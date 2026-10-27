use skrifa::raw::TableProvider;

use crate::color::rgb;
use crate::surface::Surface;
use crate::text::Font;
use crate::text::GlyphId;

pub(crate) fn has_svg_data(font: &Font, glyph: GlyphId) -> bool {
    font.font_ref()
        .svg()
        .and_then(|svg_table| svg_table.glyph_data(glyph.to_skrifa()))
        .is_ok_and(|data| data.is_some())
}

/// Draw an SVG-based glyph on a surface.
pub(crate) fn draw_glyph(
    font: Font,
    context_color: rgb::Color,
    glyph: GlyphId,
    surface: &mut Surface,
) -> Option<()> {
    let svg_data = font
        .font_ref()
        .svg()
        .and_then(|svg_table| svg_table.glyph_data(glyph.to_skrifa()))
        .ok()??;

    let upem = font.units_per_em();

    let old_fill = surface.get_fill().cloned();
    let old_stroke = surface.get_stroke().cloned();

    let fn_ = surface.sc.serialize_settings().render_svg_glyph_fn;
    let result = fn_(svg_data, context_color, glyph, (upem, upem), surface);

    surface.set_fill(old_fill);
    surface.set_stroke(old_stroke);

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_utils::ASSETS_PATH;
    use std::sync::Arc;

    #[test]
    fn svg_data_only_applies_to_covered_glyphs() {
        let data = std::fs::read(ASSETS_PATH.join("fonts/SVG_extra.ttf")).unwrap();
        let font = Font::new(Arc::new(data).into(), 0).unwrap();
        assert!(has_svg_data(&font, GlyphId::new(1)));
        assert!(!has_svg_data(&font, GlyphId::new(0)));
    }
}
