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
mod containers;
mod height_index;
mod image;
mod scroll;
mod virtual_list;
mod virtual_list_keyed;

pub use button::{ButtonExt, button};
pub use containers::{column, row, spacer, stack};
pub use image::{ImageView, image};
pub use scroll::{ScrollExt, ScrollView, scroll};
pub use virtual_list::virtual_list;
pub use virtual_list_keyed::virtual_list_keyed;
