//! The types an application names most often, in one glob import.
//!
//! The text style types come along, so naming a weight needs no other
//! crate:
//!
//! ```
//! use smudge::prelude::*;
//!
//! let heading = label("Account").text_size(22.0).weight(FontWeight::SEMI_BOLD);
//! ```

pub use smudge_core::animation::Spring;
pub use smudge_core::element::{AnyElement, Element, IntoElement, for_each, show};
pub use smudge_core::geometry::{Color, Corners, Point, Rect, Size};
pub use smudge_core::path::{Path, PathPaint, Stroke};
pub use smudge_core::reactive::prelude::*;
pub use smudge_core::reactive::{Memo, Owner, ReadSignal, RenderEffect, RwSignal, WriteSignal};
pub use smudge_core::style::{Align, Direction, Justify, Length, Overflow, pct, px};
pub use smudge_core::theme::{ColorToken, CornerRadii, RadiusToken, Theme};
pub use smudge_core::widget::{Panel, panel};
pub use smudge_core::window::Cursor;
pub use smudge_text::{
    FontWeight, Label, LabelExt, RichText, RichTextExt, SelectableExt, Span, SpanStyle,
    TextAreaExt, TextInputExt, TextLayout, TextStyle, TextSystem, current_text_style, label,
    provide_text_style, rich_text, selection_scope, text_area, text_input,
};
pub use smudge_widgets::{
    ButtonExt, Painter, ScrollView, Split, SplitExt, Toasts, button, canvas, column, image, modal,
    row, scroll, spacer, split, stack, toast_host, virtual_list,
};

#[cfg(feature = "window")]
pub use smudge_platform::{
    PlatformError, PresentStats, PresentStatsHandle, RendererChoice, WindowOptions, run, run_view,
    run_view_with, run_with,
};
