//! Theme tokens for colors and corner radii.
//!
//! A style can name a theme token instead of a literal value. The token
//! resolves to a concrete [`Color`] or radius when the node paints, so
//! switching the theme changes every node that uses a token and no other.
//! [`Theme`] holds the values the tokens resolve against; an application
//! picks the built-in light or dark theme with [`Theme::light`] and
//! [`Theme::dark`], or fills one of its own.
//!
//! ```
//! use smudge_core::geometry::Color;
//! use smudge_core::theme::{ColorToken, Theme};
//! use smudge_core::widget::panel;
//!
//! let card = panel().background(ColorToken::SURFACE);
//! let theme = Theme::light();
//! assert_eq!(theme.color(ColorToken::SURFACE), Color::WHITE);
//! ```

use std::cell::Cell;
use std::collections::HashMap;

use crate::geometry::Color;

/// The color a missing token resolves to, so the mistake stays visible on
/// screen even in a release build.
const MISSING_COLOR: Color = Color::rgb(255, 0, 255);

/// The radius a missing token resolves to.
///
/// Unlike the color fallback it is not visible on screen, since it paints
/// square corners. The debug assertion is what reports a missing radius.
const MISSING_RADIUS: f32 = 0.0;

/// The name of a color in a [`Theme`].
///
/// A token is a name, not a value: the value comes from the theme that is
/// current when the node paints. The built-in constants name the tokens every
/// theme should define.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ColorToken(
    /// The token's name, which themes key their colors by.
    pub &'static str,
);

impl ColorToken {
    /// The color behind the content of a plain surface.
    pub const SURFACE: ColorToken = ColorToken("surface");
    /// The surface color while the pointer is over a control.
    pub const SURFACE_HOVER: ColorToken = ColorToken("surface_hover");
    /// The surface color while a press is held on a control.
    pub const SURFACE_PRESSED: ColorToken = ColorToken("surface_pressed");
    /// The color of primary text.
    pub const TEXT: ColorToken = ColorToken("text");
    /// The color of secondary text, such as captions and placeholders.
    pub const TEXT_MUTED: ColorToken = ColorToken("text_muted");
    /// The color of interactive accents, such as a primary button.
    pub const ACCENT: ColorToken = ColorToken("accent");
    /// The accent color while the pointer is over a control that uses it.
    pub const ACCENT_HOVER: ColorToken = ColorToken("accent_hover");
    /// The accent color while a press is held on a control that uses it.
    pub const ACCENT_PRESSED: ColorToken = ColorToken("accent_pressed");
    /// The color of text painted over [`ColorToken::ACCENT`].
    pub const ACCENT_TEXT: ColorToken = ColorToken("accent_text");
    /// The color of borders around controls.
    pub const BORDER: ColorToken = ColorToken("border");
    /// The color of the ring around the keyboard-focused control.
    pub const FOCUS_RING: ColorToken = ColorToken("focus_ring");
}

/// The name of a corner radius in a [`Theme`].
///
/// A token is a name, not a value: the value comes from the theme that is
/// current when the node paints.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RadiusToken(
    /// The token's name, which themes key their radii by.
    pub &'static str,
);

impl RadiusToken {
    /// A small radius, for chips and small controls.
    pub const SMALL: RadiusToken = RadiusToken("small");
    /// A medium radius, for buttons and input fields.
    pub const MEDIUM: RadiusToken = RadiusToken("medium");
    /// A large radius, for cards and dialogs.
    pub const LARGE: RadiusToken = RadiusToken("large");
}

/// A color that is either a literal or a named theme token.
///
/// A literal paints the same under every theme. A token resolves against the
/// theme that is current when the node paints, so switching the theme
/// repaints it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ColorValue {
    /// A fixed color.
    Literal(Color),
    /// A color looked up in the current theme.
    Token(ColorToken),
}

impl From<Color> for ColorValue {
    fn from(color: Color) -> ColorValue {
        ColorValue::Literal(color)
    }
}

impl From<ColorToken> for ColorValue {
    fn from(token: ColorToken) -> ColorValue {
        ColorValue::Token(token)
    }
}

impl ColorValue {
    /// Resolves the value to a color.
    ///
    /// A literal is itself. A token is looked up in `theme`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when the theme does not define the token.
    /// A release build falls back to magenta, which shows the mistake on
    /// screen.
    pub fn resolve(&self, theme: &Theme) -> Color {
        match self {
            ColorValue::Literal(color) => *color,
            ColorValue::Token(token) => theme.color(*token),
        }
    }
}

/// A corner radius that is either a literal or a named theme token.
///
/// A literal is the same under every theme. A token resolves against the
/// theme that is current when the node paints, so switching the theme
/// repaints it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RadiusValue {
    /// A fixed radius in logical pixels.
    Literal(f32),
    /// A radius looked up in the current theme.
    Token(RadiusToken),
}

impl From<f32> for RadiusValue {
    fn from(radius: f32) -> RadiusValue {
        RadiusValue::Literal(radius)
    }
}

impl From<RadiusToken> for RadiusValue {
    fn from(token: RadiusToken) -> RadiusValue {
        RadiusValue::Token(token)
    }
}

impl Default for RadiusValue {
    /// A radius of zero, which gives square corners.
    fn default() -> RadiusValue {
        RadiusValue::Literal(0.0)
    }
}

impl RadiusValue {
    /// Resolves the value to a radius in logical pixels.
    ///
    /// A literal is itself. A token is looked up in `theme`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when the theme does not define the token.
    /// A release build falls back to zero, which gives square corners.
    pub fn resolve(&self, theme: &Theme) -> f32 {
        match self {
            RadiusValue::Literal(radius) => *radius,
            RadiusValue::Token(token) => theme.radius(*token),
        }
    }
}

/// The values that theme tokens resolve against.
///
/// The maps are keyed by the token names, so an application can add its own
/// tokens by inserting under names of its own. Built-in themes come from
/// [`Theme::light`] and [`Theme::dark`].
pub struct Theme {
    /// The colors, keyed by [`ColorToken`] name.
    pub colors: HashMap<&'static str, Color>,
    /// The corner radii in logical pixels, keyed by [`RadiusToken`] name.
    pub radii: HashMap<&'static str, f32>,
    /// Whether a token lookup ran since the flag was last cleared. The paint
    /// pass clears it before a widget paints and reads it after, so a node is
    /// repainted on a theme switch exactly when its paint consults the theme.
    consulted: Cell<bool>,
}

impl Clone for Theme {
    fn clone(&self) -> Theme {
        // The clone has not been consulted; only the theme a widget paints
        // with earns the flag.
        Theme {
            colors: self.colors.clone(),
            radii: self.radii.clone(),
            consulted: Cell::new(false),
        }
    }
}

impl PartialEq for Theme {
    fn eq(&self, other: &Theme) -> bool {
        // Consultation says how the theme was used, not what it holds, so a
        // theme switch ignores it.
        self.colors == other.colors && self.radii == other.radii
    }
}

impl std::fmt::Debug for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Theme")
            .field("colors", &self.colors)
            .field("radii", &self.radii)
            .finish()
    }
}

impl Theme {
    /// A light theme that defines every built-in token.
    ///
    /// ```
    /// use smudge_core::theme::{ColorToken, Theme};
    ///
    /// let theme = Theme::light();
    /// let surface = theme.color(ColorToken::SURFACE);
    /// assert!(surface.is_opaque());
    /// ```
    pub fn light() -> Theme {
        Theme::new(
            [
                (ColorToken::SURFACE, Color::WHITE),
                (ColorToken::SURFACE_HOVER, Color::rgb(240, 240, 240)),
                (ColorToken::SURFACE_PRESSED, Color::rgb(224, 224, 224)),
                (ColorToken::TEXT, Color::rgb(26, 26, 26)),
                (ColorToken::TEXT_MUTED, Color::rgb(102, 102, 102)),
                (ColorToken::ACCENT, Color::rgb(0, 95, 204)),
                (ColorToken::ACCENT_HOVER, Color::rgb(0, 84, 181)),
                (ColorToken::ACCENT_PRESSED, Color::rgb(0, 71, 153)),
                (ColorToken::ACCENT_TEXT, Color::WHITE),
                (ColorToken::BORDER, Color::rgb(204, 204, 204)),
                (ColorToken::FOCUS_RING, Color::rgb(0, 120, 215)),
            ],
            [
                (RadiusToken::SMALL, 2.0),
                (RadiusToken::MEDIUM, 4.0),
                (RadiusToken::LARGE, 8.0),
            ],
        )
    }

    /// A dark theme that defines every built-in token.
    pub fn dark() -> Theme {
        Theme::new(
            [
                (ColorToken::SURFACE, Color::rgb(30, 30, 30)),
                (ColorToken::SURFACE_HOVER, Color::rgb(42, 42, 42)),
                (ColorToken::SURFACE_PRESSED, Color::rgb(56, 56, 56)),
                (ColorToken::TEXT, Color::rgb(240, 240, 240)),
                (ColorToken::TEXT_MUTED, Color::rgb(153, 153, 153)),
                (ColorToken::ACCENT, Color::rgb(77, 163, 255)),
                (ColorToken::ACCENT_HOVER, Color::rgb(110, 180, 255)),
                (ColorToken::ACCENT_PRESSED, Color::rgb(140, 196, 255)),
                (ColorToken::ACCENT_TEXT, Color::rgb(11, 11, 11)),
                (ColorToken::BORDER, Color::rgb(68, 68, 68)),
                (ColorToken::FOCUS_RING, Color::rgb(77, 163, 255)),
            ],
            [
                (RadiusToken::SMALL, 2.0),
                (RadiusToken::MEDIUM, 4.0),
                (RadiusToken::LARGE, 8.0),
            ],
        )
    }

    /// Builds a theme from `colors` and `radii` in token order.
    fn new<const C: usize, const R: usize>(
        colors: [(ColorToken, Color); C],
        radii: [(RadiusToken, f32); R],
    ) -> Theme {
        Theme {
            colors: colors
                .into_iter()
                .map(|(token, color)| (token.0, color))
                .collect(),
            radii: radii
                .into_iter()
                .map(|(token, radius)| (token.0, radius))
                .collect(),
            consulted: Cell::new(false),
        }
    }

    /// Returns the color `token` resolves to.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when the theme does not define the token. A
    /// release build falls back to magenta, which shows the mistake on
    /// screen.
    pub fn color(&self, token: ColorToken) -> Color {
        self.consulted.set(true);
        match self.colors.get(token.0) {
            Some(color) => *color,
            None => {
                debug_assert!(
                    false,
                    "the theme defines no color token named {:?}",
                    token.0
                );
                MISSING_COLOR
            }
        }
    }

    /// Returns the radius in logical pixels that `token` resolves to.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when the theme does not define the token. A
    /// release build falls back to zero, which gives square corners.
    pub fn radius(&self, token: RadiusToken) -> f32 {
        self.consulted.set(true);
        match self.radii.get(token.0) {
            Some(radius) => *radius,
            None => {
                debug_assert!(
                    false,
                    "the theme defines no radius token named {:?}",
                    token.0
                );
                MISSING_RADIUS
            }
        }
    }

    /// Clears the consultation flag, before a widget paints.
    pub(crate) fn reset_consulted(&self) {
        self.consulted.set(false);
    }

    /// Returns whether a token lookup ran since the flag was last cleared.
    pub(crate) fn consulted(&self) -> bool {
        self.consulted.get()
    }
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
