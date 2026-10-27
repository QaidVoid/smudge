//! The types an application names most often, in one glob import.
//!
//! ```
//! use smudge::prelude::*;
//! ```

pub use smudge_core::element::{Element, IntoElement, for_each, show};
pub use smudge_core::geometry::{Color, Corners, Point, Rect, Size};
pub use smudge_core::reactive::prelude::*;
pub use smudge_core::reactive::{Memo, Owner, ReadSignal, RenderEffect, RwSignal, WriteSignal};
pub use smudge_core::style::{Align, Direction, Justify, Length, Overflow, pct, px};
pub use smudge_core::theme::{ColorToken, RadiusToken, Theme};
pub use smudge_core::widget::{Panel, panel};
pub use smudge_text::{
    Label, LabelExt, TextInputExt, TextLayout, TextStyle, TextSystem, current_text_style, label,
    provide_text_style, text_input,
};
pub use smudge_widgets::{
    ButtonExt, ScrollView, button, column, row, scroll, spacer, stack, virtual_list,
};

#[cfg(feature = "window")]
pub use smudge_platform::{
    PlatformError, PresentStats, PresentStatsHandle, RendererChoice, WindowOptions, run, run_with,
};
