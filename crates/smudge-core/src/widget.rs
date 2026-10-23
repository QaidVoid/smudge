//! The trait every widget implements.
//!
//! The tree stores widgets as `Box<dyn Widget>`, so the trait is object safe.
//! A widget never holds a reference to the tree: bindings push updates through
//! a queue, and the frame applies them.

use std::any::Any;

/// A piece of user interface owned by one tree node.
///
/// A widget holds its own state and is the only place that state lives. The
/// tree owns the widget and the [`Style`](crate::style::Style) that positions
/// and paints it.
///
/// Implementing this trait requires the type to be `'static`, because the tree
/// downcasts to the concrete type when a binding updates a widget property.
pub trait Widget: Any {}
