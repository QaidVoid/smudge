//! A node's style written as the inspector lists it: one line for each
//! property that differs from the default, in a short form.

use smudge_core::geometry::Size;
use smudge_core::style::{Edges, Length, Style};
use smudge_core::theme::{ColorValue, CornerRadii, RadiusValue};

/// Returns a line for each property of `style` that differs from the
/// default style's: its name and its value.
pub(crate) fn style_lines(style: &Style) -> Vec<(&'static str, String)> {
    let base = Style::default();
    let (layout, was) = (&style.layout, &base.layout);
    let mut lines = Vec::new();
    let mut add = |name: &'static str, changed: bool, value: String| {
        if changed {
            lines.push((name, value));
        }
    };
    add(
        "display",
        layout.display != was.display,
        format!("{:?}", layout.display),
    );
    add(
        "hidden",
        layout.hidden != was.hidden,
        layout.hidden.to_string(),
    );
    add(
        "direction",
        layout.direction != was.direction,
        format!("{:?}", layout.direction),
    );
    add("wrap", layout.wrap != was.wrap, layout.wrap.to_string());
    add("grow", layout.grow != was.grow, layout.grow.to_string());
    add(
        "shrink",
        layout.shrink != was.shrink,
        layout.shrink.to_string(),
    );
    for (name, value, default) in [
        ("basis", layout.basis, was.basis),
        ("width", layout.width, was.width),
        ("height", layout.height, was.height),
        ("min width", layout.min_width, was.min_width),
        ("min height", layout.min_height, was.min_height),
        ("max width", layout.max_width, was.max_width),
        ("max height", layout.max_height, was.max_height),
    ] {
        add(name, value != default, length(value));
    }
    add(
        "padding",
        layout.padding != was.padding,
        edges(layout.padding, length),
    );
    add(
        "margin",
        layout.margin != was.margin,
        edges(layout.margin, length),
    );
    add("gap", layout.gap != was.gap, gap(layout.gap));
    add(
        "align items",
        layout.align_items != was.align_items,
        layout
            .align_items
            .map_or_else(|| String::from("default"), |align| format!("{align:?}")),
    );
    add(
        "justify",
        layout.justify_content != was.justify_content,
        layout
            .justify_content
            .map_or_else(|| String::from("default"), |justify| format!("{justify:?}")),
    );
    add(
        "position",
        layout.position != was.position,
        format!("{:?}", layout.position),
    );
    add(
        "inset",
        layout.inset != was.inset,
        edges(layout.inset, length),
    );
    add(
        "border",
        layout.border_width != was.border_width,
        edges(layout.border_width, pixels),
    );
    add(
        "overflow",
        layout.overflow != was.overflow,
        format!("{:?}", layout.overflow),
    );
    let (paint, was) = (&style.paint, &base.paint);
    for (name, value, default) in [
        ("background", paint.background, was.background),
        ("border color", paint.border_color, was.border_color),
        ("text color", paint.text_color, was.text_color),
    ] {
        add(
            name,
            value != default,
            value.map_or_else(|| String::from("none"), color),
        );
    }
    add(
        "radius",
        paint.corner_radius != was.corner_radius,
        radii(paint.corner_radius),
    );
    add(
        "opacity",
        paint.opacity != was.opacity,
        paint.opacity.to_string(),
    );
    lines
}

/// Writes a length as `12px`, `50%` or `auto`.
fn length(length: Length) -> String {
    match length {
        Length::Px(value) => pixels(value),
        Length::Percent(value) => format!("{value}%"),
        Length::Auto => String::from("auto"),
    }
}

/// Writes logical pixels as `12px`.
fn pixels(value: f32) -> String {
    format!("{value}px")
}

/// Writes four edges as one value when they agree, as the vertical and the
/// horizontal value when the opposite edges agree, and as all four, from
/// the top clockwise, otherwise.
fn edges<T: PartialEq + Copy>(edges: Edges<T>, write: fn(T) -> String) -> String {
    let Edges {
        top,
        right,
        bottom,
        left,
    } = edges;
    if top == bottom && left == right {
        if top == left {
            write(top)
        } else {
            format!("{} {}", write(top), write(left))
        }
    } else {
        format!(
            "{} {} {} {}",
            write(top),
            write(right),
            write(bottom),
            write(left)
        )
    }
}

/// Writes a gap as one value when both ways agree, and as the gap between
/// rows and then between columns otherwise.
fn gap(gap: Size) -> String {
    if gap.width == gap.height {
        pixels(gap.width)
    } else {
        format!("{} {}", pixels(gap.height), pixels(gap.width))
    }
}

/// Writes a color as `#rrggbb`, with its alpha when it is not opaque, or as
/// the name of the theme token it reads.
fn color(color: ColorValue) -> String {
    match color {
        ColorValue::Literal(color) if color.a == 255 => {
            format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
        }
        ColorValue::Literal(color) => format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        ),
        ColorValue::Token(token) => format!("theme {}", token.0),
    }
}

/// Writes corner radii as one value when every corner agrees, and as all
/// four from the top left clockwise otherwise.
fn radii(radii: CornerRadii) -> String {
    let radius = |radius: RadiusValue| match radius {
        RadiusValue::Literal(value) => pixels(value),
        RadiusValue::Token(token) => format!("theme {}", token.0),
    };
    let corners = [
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ];
    if corners.iter().all(|corner| *corner == radii.top_left) {
        radius(radii.top_left)
    } else {
        corners.map(radius).join(" ")
    }
}

#[cfg(test)]
#[path = "styles_tests.rs"]
mod tests;
