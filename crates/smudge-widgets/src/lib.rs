//! The core widget set: layout containers, the button, the scroll view and
//! the virtual list.
//!
//! Every widget here is built from the [`Panel`](smudge_core::widget::Panel)
//! the core provides and the [`Label`](smudge_text::Label) the text crate
//! provides, styled with theme tokens by default. A theme switch therefore
//! repaints what these widgets show without any extra code.
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
mod scroll;
mod virtual_list;

pub use button::{ButtonExt, button};
pub use containers::{column, row, spacer, stack};
pub use scroll::{ScrollExt, ScrollView, scroll};
pub use virtual_list::virtual_list;
