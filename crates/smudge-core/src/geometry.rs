//! Geometry and color types shared by layout, paint and damage.
//!
//! Layout and paint work in logical pixels as [`f32`], with the origin at the
//! top left of the window and y growing downwards. Damage leaves the library as
//! [`PixelRect`] in physical pixels, so a renderer never repeats the scale
//! conversion.

/// A position in logical pixels, relative to the top left of the window.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal offset from the left edge of the window.
    pub x: f32,
    /// Vertical offset from the top edge of the window.
    pub y: f32,
}

/// A width and a height in logical pixels.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

/// An axis-aligned rectangle in logical pixels, stored as its minimum and
/// maximum corners.
///
/// `x0` and `y0` are the top left corner and `x1` and `y1` are the bottom right
/// corner. A rectangle is empty when its width or its height is not positive,
/// which includes the case where the corners are inverted.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Rect {
    /// Horizontal coordinate of the top left corner.
    pub x0: f32,
    /// Vertical coordinate of the top left corner.
    pub y0: f32,
    /// Horizontal coordinate of the bottom right corner.
    pub x1: f32,
    /// Vertical coordinate of the bottom right corner.
    pub y1: f32,
}

impl Rect {
    /// Creates a rectangle from its minimum and maximum corners.
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    /// Creates a rectangle from its top left corner and its size.
    pub fn from_origin_size(origin: Point, size: Size) -> Rect {
        Rect::new(
            origin.x,
            origin.y,
            origin.x + size.width,
            origin.y + size.height,
        )
    }

    /// Returns the horizontal extent, which is not positive when the rectangle
    /// is empty.
    pub fn width(self) -> f32 {
        self.x1 - self.x0
    }

    /// Returns the vertical extent, which is not positive when the rectangle is
    /// empty.
    pub fn height(self) -> f32 {
        self.y1 - self.y0
    }

    /// Returns the horizontal and vertical extents as a [`Size`].
    pub fn size(self) -> Size {
        Size {
            width: self.width(),
            height: self.height(),
        }
    }

    /// Returns the top left corner.
    pub fn origin(self) -> Point {
        Point {
            x: self.x0,
            y: self.y0,
        }
    }

    /// Returns `true` when the rectangle covers no area.
    ///
    /// A rectangle is empty when its width or its height is not positive, and
    /// when either extent is NaN.
    pub fn is_empty(self) -> bool {
        !(self.width() > 0.0 && self.height() > 0.0)
    }

    /// Returns the area in square logical pixels, or zero when the rectangle is
    /// empty.
    pub fn area(self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            self.width() * self.height()
        }
    }

    /// Returns `true` when the point lies inside the rectangle or on its edge.
    ///
    /// An empty rectangle contains no point.
    pub fn contains(self, point: Point) -> bool {
        !self.is_empty()
            && point.x >= self.x0
            && point.x <= self.x1
            && point.y >= self.y0
            && point.y <= self.y1
    }

    /// Returns the rectangle moved by `by`.
    pub fn translated(self, by: Point) -> Rect {
        Rect::new(
            self.x0 + by.x,
            self.y0 + by.y,
            self.x1 + by.x,
            self.y1 + by.y,
        )
    }

    /// Returns the part of the rectangle that overlaps `other`.
    ///
    /// The result is empty when the two do not overlap or when either operand
    /// is empty.
    pub fn intersect(self, other: Rect) -> Rect {
        if self.is_empty() || other.is_empty() {
            return Rect::default();
        }
        let rect = Rect::new(
            self.x0.max(other.x0),
            self.y0.max(other.y0),
            self.x1.min(other.x1),
            self.y1.min(other.y1),
        );
        if rect.is_empty() {
            Rect::default()
        } else {
            rect
        }
    }

    /// Returns the smallest rectangle covering both operands.
    ///
    /// An empty operand is ignored, so the union of an empty rectangle with
    /// `other` is `other`.
    pub fn union(self, other: Rect) -> Rect {
        match (self.is_empty(), other.is_empty()) {
            (true, true) => Rect::default(),
            (true, false) => other,
            (false, true) => self,
            (false, false) => Rect::new(
                self.x0.min(other.x0),
                self.y0.min(other.y0),
                self.x1.max(other.x1),
                self.y1.max(other.y1),
            ),
        }
    }
}

/// A rectangle in physical pixels, relative to the top left of the surface.
///
/// Every rectangle this library produces is non-empty and clamped to the
/// surface it was computed for, so `width` and `height` are at least one.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    /// Distance from the left edge of the surface, in physical pixels.
    pub x: u32,
    /// Distance from the top edge of the surface, in physical pixels.
    pub y: u32,
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
}

impl PixelRect {
    /// Returns the number of pixels the rectangle covers.
    pub fn area(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Returns the smallest rectangle covering both operands.
    ///
    /// An operand with no area is ignored, so the union of
    /// `PixelRect::default()` with `other` is `other`.
    pub fn union(self, other: PixelRect) -> PixelRect {
        if other.area() == 0 {
            return self;
        }
        if self.area() == 0 {
            return other;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let y1 = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));
        PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }

    /// Returns the overlapping part of two rectangles, or `None` when they do
    /// not overlap.
    pub fn intersect(self, other: PixelRect) -> Option<PixelRect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self
            .x
            .saturating_add(self.width)
            .min(other.x.saturating_add(other.width));
        let y1 = self
            .y
            .saturating_add(self.height)
            .min(other.y.saturating_add(other.height));
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        Some(PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }

    /// Returns `true` when `other` lies entirely inside this rectangle.
    ///
    /// The edges are inclusive, so a rectangle with no width or height counts
    /// as inside when it sits on the boundary.
    pub fn contains_rect(self, other: PixelRect) -> bool {
        let outer_x1 = u64::from(self.x) + u64::from(self.width);
        let outer_y1 = u64::from(self.y) + u64::from(self.height);
        u64::from(other.x) >= u64::from(self.x)
            && u64::from(other.y) >= u64::from(self.y)
            && u64::from(other.x) + u64::from(other.width) <= outer_x1
            && u64::from(other.y) + u64::from(other.height) <= outer_y1
    }

    /// Returns the smallest pixel rectangle covering every physical pixel the
    /// logical rectangle touches, clamped to the surface.
    ///
    /// The minimum corner is rounded down and the maximum corner is rounded up,
    /// so a rectangle that covers part of a pixel still claims that pixel.
    /// Returns `None` when the result would be empty, which includes a
    /// rectangle entirely outside the surface.
    ///
    /// NaN and infinite coordinates, and a scale that is not positive and
    /// finite, are treated as empty.
    pub fn from_logical(
        rect: Rect,
        scale: f32,
        surface_width: u32,
        surface_height: u32,
    ) -> Option<PixelRect> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        if !rect.x0.is_finite()
            || !rect.y0.is_finite()
            || !rect.x1.is_finite()
            || !rect.y1.is_finite()
        {
            return None;
        }
        let max_x = surface_width as f32;
        let max_y = surface_height as f32;
        let x0 = (rect.x0 * scale).floor().clamp(0.0, max_x) as u32;
        let y0 = (rect.y0 * scale).floor().clamp(0.0, max_y) as u32;
        let x1 = (rect.x1 * scale).ceil().clamp(0.0, max_x) as u32;
        let y1 = (rect.y1 * scale).ceil().clamp(0.0, max_y) as u32;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }
}

/// A straight-alpha sRGB color.
///
/// Channels are not premultiplied. Renderers convert to whatever their own
/// surface expects.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel, where 0 is fully transparent and 255 fully opaque.
    pub a: u8,
}

impl Color {
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    /// Creates a fully opaque color from its red, green and blue channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// Creates a color from its red, green, blue and alpha channels.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    /// Returns `true` when the color is fully opaque.
    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }
}

/// The corner radii of a rounded rectangle, in logical pixels.
///
/// A radius of zero gives a square corner.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Corners {
    /// Radius of the top left corner.
    pub top_left: f32,
    /// Radius of the top right corner.
    pub top_right: f32,
    /// Radius of the bottom right corner.
    pub bottom_right: f32,
    /// Radius of the bottom left corner.
    pub bottom_left: f32,
}

impl Corners {
    /// Square corners.
    pub const ZERO: Corners = Corners::all(0.0);

    /// A radius of `radius` logical pixels on every corner.
    pub const fn all(radius: f32) -> Corners {
        Corners {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
