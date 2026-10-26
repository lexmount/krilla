use super::*;
use crate::color::rgb;
use crate::page::PageSettings;
use crate::paint::{LinearGradient, SpreadMethod, Stop};
use crate::text::{KrillaGlyph, Tag};
use crate::Document;
use skrifa::MetadataProvider;

const PROBE: &[u8] = include_bytes!("../../../tests/fixtures/expanded-1000.ttf");
const PROBE_250: &[u8] = include_bytes!("../../../tests/fixtures/expanded-250.ttf");

fn font(data: &[u8]) -> Font {
    Font::new(data.to_vec().into(), 0).unwrap()
}

fn glyphs(font: &Font, text: &str) -> Vec<KrillaGlyph> {
    text.char_indices()
        .map(|(offset, ch)| KrillaGlyph {
            glyph_id: GlyphId::new(font.font_ref().charmap().map(ch).unwrap().to_u32()),
            text_range: offset..offset + ch.len_utf8(),
            x_advance: 0.8,
            y_advance: 0.0,
            x_offset: 0.0,
            y_offset: 0.0,
            location: None,
        })
        .collect()
}

fn stroke() -> Stroke {
    Stroke {
        paint: rgb::Color::new(255, 0, 0).into(),
        width: 2.0,
        line_join: LineJoin::Miter,
        miter_limit: 4.0,
        ..Default::default()
    }
}

fn parse(pdf: Vec<u8>) -> lopdf::Document {
    lopdf::Document::load_mem(&pdf).unwrap()
}

fn extracted(doc: &lopdf::Document) -> String {
    doc.extract_text(&doc.get_pages().keys().copied().collect::<Vec<_>>())
        .unwrap()
        .split_whitespace()
        .collect::<String>()
}

#[test]
fn expansion_rejects_invalid_amounts() {
    for value in [-0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
        assert!(GlyphExpansion::new(value, 0.3).is_none());
        assert!(GlyphExpansion::new(0.3, value).is_none());
    }
    assert_eq!(
        GlyphExpansion::default(),
        GlyphExpansion::new(-0.0, 0.0).unwrap()
    );
}

#[test]
fn expanded_geometry_keeps_surface_units_and_both_axes() {
    for data in [PROBE, PROBE_250] {
        let font = font(data);
        let glyphs = glyphs(&font, "R");
        for size in [16.0, 20.0, 100.0] {
            for (x, y) in [(0.3, 0.0), (0.0, 0.24), (0.3, 0.24)] {
                let mut doc = Document::new();
                let mut page = doc.start_page();
                let mut surface = page.surface();
                surface.set_stroke(Some(stroke()));
                surface.draw_expanded_glyphs(
                    Point::from_xy(20.0, 120.0),
                    &glyphs,
                    font.clone(),
                    "R",
                    size,
                    GlyphExpansion::new(x, y).unwrap(),
                    GlyphPaintOrder::FillStroke,
                    false,
                );
                let cache = &surface.sc.expanded_glyphs;
                assert_eq!(cache.paths.len(), 1);
                let bounds = cache
                    .paths
                    .values()
                    .next()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .0
                    .bounds();
                for (actual, expected) in
                    [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()]
                        .into_iter()
                        .zip([-x, -size * 0.8 - y, size * 0.6 + x, y])
                {
                    assert!(
                        (actual - expected).abs() < 0.02,
                        "size={size}, x={x}, y={y}, actual={actual}, expected={expected}"
                    );
                }
                surface.finish();
                page.finish();
                assert_eq!(extracted(&parse(doc.finish().unwrap())), "R");
            }
        }
    }
}

#[test]
fn both_paint_orders_and_hollow_fills_keep_one_text_copy() {
    let font = font(PROBE);
    for order in [GlyphPaintOrder::FillStroke, GlyphPaintOrder::StrokeFill] {
        for fill in [
            None,
            Some(Fill::default()),
            Some(Fill {
                opacity: NormalizedF32::new(0.5).unwrap(),
                ..Default::default()
            }),
        ] {
            for expansion in [
                GlyphExpansion::default(),
                GlyphExpansion::new(0.3, 0.24).unwrap(),
            ] {
                let mut doc = Document::new();
                let mut page = doc.start_page();
                let mut surface = page.surface();
                let stroke = stroke();
                surface.set_fill(fill.clone());
                surface.set_stroke(Some(stroke.clone()));
                surface.push_transform(&Transform::from_row(1.0, 0.1, 0.2, 1.0, 0.0, 0.0));
                let mut glyphs = glyphs(&font, "RB");
                glyphs[1].y_offset = 0.2;
                surface.draw_expanded_glyphs(
                    Point::from_xy(20.0, 120.0),
                    &glyphs,
                    font.clone(),
                    "RB",
                    20.0,
                    expansion,
                    order,
                    false,
                );
                if expansion.is_zero() {
                    assert!(surface.sc.expanded_glyphs.paths.is_empty());
                    assert!(surface.sc.expanded_glyphs.graphics.is_empty());
                }
                assert_eq!(surface.get_fill(), fill.as_ref());
                assert_eq!(surface.get_stroke(), Some(&stroke));
                surface.pop();
                surface.finish();
                page.finish();
                assert_eq!(
                    extracted(&parse(doc.finish().unwrap())),
                    "RB",
                    "order={order:?}, fill={fill:?}"
                );
            }
        }
    }
}

#[test]
fn contours_and_graphics_are_reused_across_pages_and_runs() {
    let font = font(PROBE);
    let glyphs = glyphs(&font, "RRBB");
    let mut doc = Document::new();
    for _ in 0..2 {
        let mut page = doc.start_page_with(PageSettings::from_wh(200.0, 200.0).unwrap());
        let mut surface = page.surface();
        surface.set_fill(Some(Fill::default()));
        surface.set_stroke(Some(stroke()));
        for y in [60.0, 120.0] {
            surface.draw_expanded_glyphs(
                Point::from_xy(20.0, y),
                &glyphs,
                font.clone(),
                "RRBB",
                20.0,
                GlyphExpansion::new(0.3, 0.24).unwrap(),
                GlyphPaintOrder::FillStroke,
                false,
            );
            assert_eq!(surface.sc.expanded_glyphs.paths.len(), 2);
            assert_eq!(surface.sc.expanded_glyphs.graphics.len(), 4);
        }
        surface.finish();
        page.finish();
    }
    let doc = parse(doc.finish().unwrap());
    assert_eq!(extracted(&doc), "RRBB".repeat(4));
    assert_eq!(
        doc.objects
            .values()
            .filter(|object| object.as_stream().is_ok_and(|stream| stream
                .dict
                .get(b"Subtype")
                .and_then(|value| value.as_name())
                .ok()
                == Some(b"Form")))
            .count(),
        4
    );
}

#[test]
fn gradients_keep_surface_space_and_do_not_use_local_graphics() {
    let font = font(PROBE);
    let mut doc = Document::new();
    let mut page = doc.start_page();
    let mut surface = page.surface();
    surface.set_fill(Some(Fill {
        paint: LinearGradient {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 0.0,
            transform: Default::default(),
            spread_method: SpreadMethod::Pad,
            stops: vec![
                Stop {
                    offset: NormalizedF32::ZERO,
                    color: rgb::Color::new(255, 0, 0).into(),
                    opacity: NormalizedF32::ONE,
                },
                Stop {
                    offset: NormalizedF32::ONE,
                    color: rgb::Color::new(0, 0, 255).into(),
                    opacity: NormalizedF32::ONE,
                },
            ],
            anti_alias: false,
        }
        .into(),
        ..Default::default()
    }));
    surface.draw_expanded_glyphs(
        Point::from_xy(20.0, 120.0),
        &glyphs(&font, "RR"),
        font.clone(),
        "RR",
        20.0,
        GlyphExpansion::new(0.3, 0.24).unwrap(),
        GlyphPaintOrder::FillStroke,
        false,
    );
    assert_eq!(surface.sc.expanded_glyphs.paths.len(), 1);
    assert!(surface.sc.expanded_glyphs.graphics.is_empty());
    surface.finish();
    page.finish();
    assert_eq!(extracted(&parse(doc.finish().unwrap())), "RR");
}

#[test]
fn cache_distinguishes_font_variations_and_font_size() {
    let data = include_bytes!("../../../../../assets/fonts/Cantarell-VF.otf");
    let mut doc = Document::new();
    let mut page = doc.start_page();
    let mut surface = page.surface();
    surface.set_fill(Some(Fill {
        opacity: NormalizedF32::new(0.5).unwrap(),
        ..Default::default()
    }));
    for weight in [400.0, 700.0] {
        let font =
            Font::new_variable(data.to_vec().into(), 0, &[(Tag::new(b"wght"), weight)]).unwrap();
        for size in [20.0, 40.0] {
            surface.draw_expanded_glyphs(
                Point::from_xy(20.0, 120.0),
                &glyphs(&font, "A"),
                font.clone(),
                "A",
                size,
                GlyphExpansion::new(0.3, 0.24).unwrap(),
                GlyphPaintOrder::FillStroke,
                false,
            );
        }
    }
    assert_eq!(surface.sc.expanded_glyphs.paths.len(), 4);
    assert_eq!(surface.sc.expanded_glyphs.graphics.len(), 4);
    surface.finish();
    page.finish();
    assert_eq!(extracted(&parse(doc.finish().unwrap())), "AAAA");
}

#[test]
fn outlined_expansion_emits_no_searchable_text() {
    let font = font(PROBE);
    let mut doc = Document::new();
    let mut page = doc.start_page();
    let mut surface = page.surface();
    surface.set_fill(Some(Fill::default()));
    surface.draw_expanded_glyphs(
        Point::from_xy(20.0, 120.0),
        &glyphs(&font, "R"),
        font.clone(),
        "R",
        20.0,
        GlyphExpansion::new(0.3, 0.24).unwrap(),
        GlyphPaintOrder::FillStroke,
        true,
    );
    surface.finish();
    page.finish();
    let doc = parse(doc.finish().unwrap());
    assert!(doc.get_page_fonts(doc.get_pages()[&1]).unwrap().is_empty());
}

#[test]
fn color_glyphs_keep_their_representation_and_single_text_mapping() {
    let font = font(include_bytes!(
        "../../../../../assets/fonts/NotoColorEmoji.COLR.subset.ttf"
    ));
    let mut pdf = Document::new();
    let mut page = pdf.start_page();
    let mut surface = page.surface();
    surface.set_fill(Some(Fill {
        paint: rgb::Color::new(0, 0, 255).into(),
        ..Default::default()
    }));
    surface.set_stroke(Some(stroke()));
    surface.draw_expanded_glyphs(
        Point::from_xy(20.0, 120.0),
        &glyphs(&font, "😄"),
        font.clone(),
        "😄",
        20.0,
        GlyphExpansion::new(0.3, 0.24).unwrap(),
        GlyphPaintOrder::StrokeFill,
        false,
    );
    assert_eq!(surface.sc.expanded_glyphs.paths.len(), 1);
    assert!(surface
        .sc
        .expanded_glyphs
        .paths
        .values()
        .all(Option::is_none));
    assert!(surface.sc.expanded_glyphs.graphics.is_empty());
    surface.finish();
    page.finish();
    let mut doc = parse(pdf.finish().unwrap());
    // Read Type3 Unicode maps rather than lopdf's preferred Differences names.
    for object in doc.objects.values_mut() {
        if let Ok(dict) = object.as_dict_mut() {
            if dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Type3") {
                dict.remove(b"Encoding");
            }
        }
    }
    assert_eq!(extracted(&doc), "😄");
}
