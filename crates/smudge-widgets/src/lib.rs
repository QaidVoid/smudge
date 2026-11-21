//! The core widget set: layout containers, the button, the image, the scroll
//! view and the virtual list.
//!
//! The widgets here are built from the [`Panel`](smudge_core::widget::Panel)
//! the core provides, the [`Label`](smudge_text::Label) the text crate
//! provides, and the [`Widget`](smudge_core::widget::Widget) trait directly,
//! styled with theme tokens by default. A theme switch therefore repaints
//! what these widgets show without any extra code.
//!
//! ```
//! use smudge_core::geometry::Size;
//! use smudge_core::style::px;
//! use smudge_widgets::{ButtonExt, button, column, scroll, spacer};
//!
//! let toolbar = column()
//!     .width(px(200.0))
//!     .gap(Size {
//!         width: 8.0,
//!         height: 8.0,
//!     })
//!     .child(button("Save"))
//!     .child(button("Cancel").primary())
//!     .child(spacer())
//!     .child(scroll(column()).height(px(120.0)));
//! ```

mod button;
mod canvas;
mod containers;
mod controls;
mod dropdown;
mod height_index;
mod image;
mod marks;
mod menu;
mod modal;
mod scroll;
mod split;
mod tabs;
mod toasts;
mod virtual_list;
mod virtual_list_keyed;

pub use button::{ButtonExt, button};
pub use canvas::{Canvas, Painter, canvas};
pub use containers::{column, row, spacer, stack};
pub use controls::{
    RadioGroup, RangeSlider, SliderArea, checkbox, number_field, radio_group, range_slider, slider,
    switch,
};
pub use dropdown::{TooltipExt, dropdown};
pub use image::{ImageView, image};
pub use menu::{
    ContextMenuExt, MenuBar, MenuItem, menu, menu_bar, menu_check_item, menu_item, menu_separator,
    submenu,
};
pub use modal::{Modal, modal};
pub use scroll::{ScrollAxis, ScrollExt, ScrollView, scroll};
pub use split::{Split, SplitExt, split};
pub use tabs::{Tabs, tabs};
pub use toasts::{MAX_TOASTS, TOAST_DURATION, Toasts, toast_host};
pub use virtual_list::virtual_list;
pub use virtual_list_keyed::{virtual_column_keyed, virtual_list_keyed};

/// Creates a signal the library keeps for itself, such as a widget's own
/// state.
///
/// It passes no caller on, unlike the public constructors that call it, so
/// hot reload files the signal under this crate rather than under the
/// app's code, where it would take the place of one of the app's signals.
pub(crate) fn signal<T: 'static>(value: T) -> smudge_core::reactive::RwSignal<T> {
    smudge_core::reactive::RwSignal::new(value)
}
