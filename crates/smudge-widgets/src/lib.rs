//! The core widget set: layout containers and the button.
//!
//! Every widget here is built from the [`Panel`](smudge_core::widget::Panel)
//! the core provides and the [`Label`](smudge_text::Label) the text crate
//! provides, styled with theme tokens by default. A theme switch therefore
//! repaints what these widgets show without any extra code.
//!
//! ```
//! use smudge_core::geometry::Size;
//! use smudge_core::style::px;
//! use smudge_widgets::ButtonExt;
//! use smudge_widgets::{button, column, spacer};
//!
//! let toolbar = column()
//!     .width(px(200.0))
//!     .gap(Size {
//!         width: 8.0,
//!         height: 8.0,
//!     })
//!     .child(button("Save"))
//!     .child(button("Cancel").primary())
//!     .child(spacer());
//! ```

mod button;
mod containers;

pub use button::{ButtonExt, button};
pub use containers::{column, row, spacer, stack};
