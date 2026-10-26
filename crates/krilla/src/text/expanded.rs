use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{BezPath, Diagonal2, Join, PathEl};
use skrifa::instance::Size;
use skrifa::outline::{pen::PathElement, DrawSettings};
use tiny_skia_path::FiniteF32;

use crate::geom::{Path, Point, Transform};
use crate::graphic::Graphic;
use crate::num::NormalizedF32;
use crate::paint::{Fill, InnerPaint, LineJoin, Stroke};
use crate::surface::Surface;
use crate::text::glyph::{colr, svg};
use crate::text::{Font, Glyph, GlyphId};

/// Horizontal and vertical expansion of a monochrome glyph contour.
///
/// Amounts are local surface units at the requested font size, not font units.
/// Expansion uses miter joins with limit 4 and a 0.1-unit approximation tolerance.
/// It is independent of the authored stroke's width and join settings.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GlyphExpansion {
    x: FiniteF32,
    y: FiniteF32,
}

impl GlyphExpansion {
    /// Create a nonnegative, finite expansion. Returns `None` for invalid amounts.
    pub fn new(x: f32, y: f32) -> Option<Self> {
        if x < 0.0 || y < 0.0 || !(2.0 * x).is_finite() || !(2.0 * y).is_finite() {
            return None;
        }
        Some(Self {
            x: FiniteF32::new(x.max(0.0))?,
            y: FiniteF32::new(y.max(0.0))?,
        })
    }

    /// Horizontal expansion in local surface units.
    pub fn x(self) -> f32 {
        self.x.get()
    }

    /// Vertical expansion in local surface units.
    pub fn y(self) -> f32 {
        self.y.get()
    }

    fn is_zero(self) -> bool {
        self.x() == 0.0 && self.y() == 0.0
    }
}

impl Default for GlyphExpansion {
    fn default() -> Self {
        Self::new(0.0, 0.0).unwrap()
    }
}

/// The order of the visible fill and stroke passes.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub enum GlyphPaintOrder {
    /// Fill the glyphs, then stroke them.
    #[default]
    FillStroke,
    /// Stroke the glyphs, then fill them.
    StrokeFill,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct GeometryKey {
    font: Font,
    glyph: GlyphId,
    size: FiniteF32,
    expansion: GlyphExpansion,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum GlyphPaint {
    Fill(Fill),
    Stroke(Stroke),
}

impl GlyphPaint {
    fn is_solid(&self) -> bool {
        matches!(
            match self {
                Self::Fill(p) => &p.paint.0,
                Self::Stroke(p) => &p.paint.0,
            },
            InnerPaint::Color(_)
        )
    }

    fn set(&self, surface: &mut Surface<'_>) {
        match self {
            Self::Fill(fill) => {
                surface.set_fill(Some(fill.clone()));
                surface.set_stroke(None);
            }
            Self::Stroke(stroke) => {
                surface.set_fill(None);
                surface.set_stroke(Some(stroke.clone()));
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct ExpandedGlyphCache {
    paths: HashMap<GeometryKey, Option<Arc<Path>>>,
    graphics: HashMap<(GeometryKey, GlyphPaint), Graphic>,
}

fn expanded_path(key: &GeometryKey) -> Option<Arc<Path>> {
    let font = &key.font;
    // Representation choice precedes outline preparation: a COLR glyph's base
    // outline does not describe its artwork, even when it is nonempty.
    if colr::has_colr_data(font, key.glyph) || svg::has_svg_data(font, key.glyph) {
        return None;
    }
    #[cfg(feature = "raster-images")]
    if crate::text::glyph::bitmap::has_bitmap_data(font, key.glyph) {
        return None;
    }
    let glyph = font.outline_glyphs().get(key.glyph.to_skrifa())?;
    let mut elements = Vec::new();
    glyph
        .draw(
            DrawSettings::unhinted(Size::new(key.size.get()), font.location_ref()),
            &mut elements,
        )
        .ok()?;
    let point = |x, y| (f64::from(x), f64::from(y)).into();
    let path = BezPath::from_vec(
        elements
            .into_iter()
            .map(|el| match el {
                PathElement::MoveTo { x, y } => PathEl::MoveTo(point(x, y)),
                PathElement::LineTo { x, y } => PathEl::LineTo(point(x, y)),
                PathElement::QuadTo { cx0, cy0, x, y } => {
                    PathEl::QuadTo(point(cx0, cy0), point(x, y))
                }
                PathElement::CurveTo {
                    cx0,
                    cy0,
                    cx1,
                    cy1,
                    x,
                    y,
                } => PathEl::CurveTo(point(cx0, cy0), point(cx1, cy1), point(x, y)),
                PathElement::Close => PathEl::ClosePath,
            })
            .collect(),
    );
    let path = kurbo::expand_path(
        &path,
        Diagonal2::new(f64::from(key.expansion.x()), f64::from(key.expansion.y())),
        Join::Miter,
        4.0,
        0.1,
    );
    let mut builder = crate::geom::PathBuilder::new();
    for element in path.elements() {
        match element {
            PathEl::MoveTo(p) => builder.move_to(p.x as f32, -p.y as f32),
            PathEl::LineTo(p) => builder.line_to(p.x as f32, -p.y as f32),
            PathEl::QuadTo(p1, p) => {
                builder.quad_to(p1.x as f32, -p1.y as f32, p.x as f32, -p.y as f32)
            }
            PathEl::CurveTo(p1, p2, p) => builder.cubic_to(
                p1.x as f32,
                -p1.y as f32,
                p2.x as f32,
                -p2.y as f32,
                p.x as f32,
                -p.y as f32,
            ),
            PathEl::ClosePath => builder.close(),
        }
    }
    Some(Arc::new(builder.finish()?))
}

fn graphic(
    surface: &mut Surface<'_>,
    key: &GeometryKey,
    path: &Path,
    paint: &GlyphPaint,
) -> Graphic {
    let cache_key = (key.clone(), paint.clone());
    if let Some(graphic) = surface.sc.expanded_glyphs.graphics.get(&cache_key) {
        return graphic.clone();
    }
    let mut stream = surface.stream_builder();
    let mut glyph = stream.surface();
    paint.set(&mut glyph);
    glyph.draw_path(path);
    glyph.finish();
    let graphic = Graphic::new(stream.finish(), false);
    surface
        .sc
        .expanded_glyphs
        .graphics
        .insert(cache_key, graphic.clone());
    graphic
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    surface: &mut Surface<'_>,
    start: Point,
    glyphs: &[impl Glyph],
    font: Font,
    text: &str,
    font_size: f32,
    expansion: GlyphExpansion,
    order: GlyphPaintOrder,
    outlined: bool,
) {
    let Some(size) = FiniteF32::new(font_size).filter(|_| font_size > 0.0) else {
        return;
    };
    if glyphs.is_empty() {
        return;
    }
    let initial_location = surface.sc.location;
    let original_fill = surface.get_fill().cloned();
    let original_stroke = surface.get_stroke().cloned();
    let fill = original_fill
        .clone()
        .or_else(|| original_stroke.is_none().then(Fill::default));
    let stroke = original_stroke.clone();
    let synthesized = !expansion.is_zero();
    let fallback_amount = expansion.x().max(expansion.y());
    let native_fill = !synthesized
        || (expansion.x() == expansion.y()
            && fill.as_ref().is_some_and(|f| {
                f.opacity == NormalizedF32::ONE && matches!(&f.paint.0, InnerPaint::Color(_))
            }));
    let need_paths =
        synthesized && (stroke.is_some() || (fill.is_some() && (outlined || !native_fill)));
    let keys: Vec<_> = if need_paths {
        glyphs
            .iter()
            .map(|g| GeometryKey {
                font: font.clone(),
                glyph: g.glyph_id(),
                size,
                expansion,
            })
            .collect()
    } else {
        Vec::new()
    };
    let paths: Vec<_> = keys
        .iter()
        .map(|key| {
            need_paths
                .then(|| {
                    surface
                        .sc
                        .expanded_glyphs
                        .paths
                        .entry(key.clone())
                        .or_insert_with(|| expanded_path(key))
                        .clone()
                })
                .flatten()
        })
        .collect();
    let any_path = paths.iter().any(Option::is_some);
    let path_fill = fill.is_some() && (outlined || !native_fill) && any_path;
    let path_stroke = stroke.is_some() && any_path;
    let semantic_copy = !outlined && (path_fill || (fill.is_none() && path_stroke));
    let draw_native =
        |surface: &mut Surface<'_>, fill: Option<Fill>, stroke: Option<Stroke>, outlined| {
            surface.set_fill(fill);
            surface.set_stroke(stroke);
            surface.sc.location = initial_location;
            surface.draw_glyphs(start, glyphs, font.clone(), text, font_size, outlined);
            surface.sc.location = initial_location;
        };
    if semantic_copy {
        let context_color = fill
            .as_ref()
            .and_then(|f| f.paint.as_rgb())
            .or_else(|| stroke.as_ref().and_then(|s| s.paint.as_rgb()))
            .unwrap_or_default();
        draw_native(
            surface,
            Some(Fill {
                paint: context_color.into(),
                opacity: NormalizedF32::ZERO,
                ..Fill::default()
            }),
            None,
            false,
        );
    }
    let fake_weight = |fill: &Fill| Stroke {
        paint: fill.paint.clone(),
        opacity: fill.opacity,
        width: 2.0 * fallback_amount,
        line_join: LineJoin::Miter,
        miter_limit: 4.0,
        ..Default::default()
    };
    let draw_paths = |surface: &mut Surface<'_>, paint: GlyphPaint| {
        let (mut x, mut y) = (start.x, start.y);
        for (i, glyph) in glyphs.iter().enumerate() {
            surface.sc.location = glyph.location().or(initial_location);
            if let Some(path) = &paths[i] {
                let transform = Transform::from_translate(
                    x + glyph.x_offset(font_size),
                    y - glyph.y_offset(font_size),
                );
                if paint.is_solid() {
                    let graphic = graphic(surface, &keys[i], path, &paint);
                    surface.push_transform(&transform);
                    surface.draw_graphic(graphic);
                    surface.pop();
                } else if let Some(path) = (Path(path.0.clone())).transform(transform) {
                    // Keep paints in run/surface space instead of repeating the
                    // gradient or pattern separately in every glyph's local space.
                    paint.set(surface);
                    surface.draw_path(&path);
                }
            } else {
                // Preserve the established native fallback for color, image,
                // empty or unavailable outlines, including its synthetic width.
                match &paint {
                    GlyphPaint::Fill(fill) => {
                        surface.set_fill(Some(fill.clone()));
                        surface.set_stroke(synthesized.then(|| fake_weight(fill)));
                    }
                    GlyphPaint::Stroke(stroke) => {
                        surface.set_fill(None);
                        surface.set_stroke(Some(Stroke {
                            width: stroke.width + 2.0 * fallback_amount,
                            ..stroke.clone()
                        }));
                    }
                }
                surface.draw_glyphs(
                    Point::from_xy(x, y),
                    &glyphs[i..i + 1],
                    font.clone(),
                    text,
                    font_size,
                    true,
                );
            }
            x += glyph.x_advance(font_size);
            y -= glyph.y_advance(font_size);
        }
        surface.sc.location = initial_location;
    };
    let draw_stroke = |surface: &mut Surface<'_>| {
        if let Some(stroke) = &stroke {
            if path_stroke {
                draw_paths(surface, GlyphPaint::Stroke(stroke.clone()));
            } else {
                draw_native(
                    surface,
                    None,
                    Some(Stroke {
                        width: stroke.width + 2.0 * fallback_amount,
                        ..stroke.clone()
                    }),
                    outlined || fill.is_some() || semantic_copy,
                );
            }
        }
    };
    if order == GlyphPaintOrder::StrokeFill || fill.is_none() {
        draw_stroke(surface);
    }
    if let Some(fill) = &fill {
        if path_fill {
            draw_paths(surface, GlyphPaint::Fill(fill.clone()));
        } else {
            draw_native(
                surface,
                Some(fill.clone()),
                synthesized.then(|| fake_weight(fill)),
                outlined || semantic_copy,
            );
        }
        if order == GlyphPaintOrder::FillStroke {
            draw_stroke(surface);
        }
    }
    surface.set_fill(original_fill);
    surface.set_stroke(original_stroke);
    surface.sc.location = initial_location;
}

#[cfg(test)]
mod tests;
