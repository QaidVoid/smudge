//! Overlays: subtrees laid out, painted and hit tested over the whole
//! window, outside every clip of the node that opened them.
//!
//! An overlay's top node has its host, the node that opened it, as its
//! parent, so keys bubble to the host and its owner hangs under the host's,
//! but the host does not list it among its children. Layout, the display
//! list, hit testing and the accessibility tree reach it through
//! [`WidgetTree::overlays`] instead, after the main tree. Decision 0015
//! records the model.

use crate::geometry::{Point, Rect, Size};
use crate::input::{Key, KeyEvent};
use crate::reactive::{RwSignal, prelude::*};

use super::{WidgetId, WidgetTree};

/// Where an overlay goes, relative to the node it belongs to.
///
/// Each placement flips to the other side when the window has no room for
/// the overlay on the first and does on the other, and then shifts the
/// overlay along as little as it takes to keep it inside the window.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Placement {
    /// Below the node, its left edge on the node's left edge, as a dropdown
    /// opens.
    Below,
    /// Above the node, its left edge on the node's left edge.
    Above,
    /// Right of the node, its top on the node's top, as a submenu opens.
    After,
    /// With its top left corner at this point, relative to the top left of
    /// the node's rectangle, as a context menu opens at the pointer. It
    /// opens up and to the left of the point where the window has no room
    /// below and to the right.
    At(Point),
    /// Over the whole window, its top node laid out at the window's size,
    /// as a dialog's backdrop is.
    Cover,
    /// In the window's bottom right corner, as toasts are. Give the top node
    /// a margin to keep it off the edges.
    BottomRight,
}

/// Which layer an overlay draws in, and so what it draws above and below.
///
/// From the bottom: the app's tree with its popups, then its tooltips,
/// then each open dialog in the order it opened, each with its own popups
/// and tooltips above it, and the toasts on top of everything. An overlay
/// opened from inside a dialog joins that dialog's popups and tooltips, so
/// a dropdown in a dialog draws above the dialog and below a dialog opened
/// after it. Within one layer, the overlay opened last draws on top.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Layer {
    /// A popup, a menu or a dropdown's list.
    #[default]
    Popup,
    /// A tooltip.
    Tooltip,
    /// A dialog.
    Dialog,
    /// A toast.
    Toast,
}

impl Placement {
    /// Returns whether the overlay hangs off the node's rectangle, so a
    /// press on the node itself is not a press outside the overlay.
    fn anchored_to_node(self) -> bool {
        !matches!(
            self,
            Placement::At(_) | Placement::Cover | Placement::BottomRight
        )
    }

    /// Returns the top left corner of an overlay of `size` placed this way
    /// against `anchor`, the node's rectangle in window coordinates, inside a
    /// window of `viewport`.
    pub(crate) fn place(self, anchor: Rect, size: Size, viewport: Size) -> Point {
        let (width, height) = (size.width, size.height);
        let fits_below = |y: f32| y + height <= viewport.height;
        let (x, y) = match self {
            Placement::Below => {
                let below = anchor.y1;
                let above = anchor.y0 - height;
                let y = if !fits_below(below) && above >= 0.0 {
                    above
                } else {
                    below
                };
                (anchor.x0, y)
            }
            Placement::Above => {
                let above = anchor.y0 - height;
                let y = if above < 0.0 && fits_below(anchor.y1) {
                    anchor.y1
                } else {
                    above
                };
                (anchor.x0, y)
            }
            Placement::After => {
                let after = anchor.x1;
                let before = anchor.x0 - width;
                let x = if after + width > viewport.width && before >= 0.0 {
                    before
                } else {
                    after
                };
                (x, anchor.y0)
            }
            Placement::Cover => (0.0, 0.0),
            Placement::BottomRight => (viewport.width - width, viewport.height - height),
            Placement::At(at) => {
                let (px, py) = (anchor.x0 + at.x, anchor.y0 + at.y);
                let x = if px + width > viewport.width && px - width >= 0.0 {
                    px - width
                } else {
                    px
                };
                let y = if !fits_below(py) && py - height >= 0.0 {
                    py - height
                } else {
                    py
                };
                (x, y)
            }
        };
        Point {
            x: x.min(viewport.width - width).max(0.0),
            y: y.min(viewport.height - height).max(0.0),
        }
    }
}

/// How an overlay opens: where it goes, what keeps it open, and whether the
/// pointer passes through it.
#[derive(Clone)]
pub(crate) struct Opening {
    /// Where it goes relative to its host.
    pub(crate) placement: Placement,
    /// The signal that keeps it open.
    pub(crate) open: RwSignal<bool>,
    /// Whether the pointer passes through it to what lies under it.
    pub(crate) passes_pointer: bool,
    /// The layer it draws in.
    pub(crate) layer: Layer,
}

/// An overlay the tree shows.
pub(crate) struct Overlay {
    /// The overlay's top node, whose parent is its host.
    pub(crate) root: WidgetId,
    /// Where it goes relative to its host.
    pub(crate) placement: Placement,
    /// The signal that keeps it open, which Escape and a press outside it
    /// set to `false`.
    pub(crate) open: RwSignal<bool>,
    /// The top left corner it was last placed at, in window coordinates.
    pub(crate) origin: Point,
    /// Whether the pointer passes through it to what lies under it, as it
    /// does through a tooltip.
    pub(crate) passes_pointer: bool,
    /// The layer it draws in.
    pub(crate) layer: Layer,
    /// Where it sorts among the overlays: the dialog it belongs to, by the
    /// order the dialogs opened, zero for the app's tree and the most for a
    /// toast, then its rank in that dialog's layers, then the order it
    /// opened in.
    pub(crate) order: (u64, u8, u64),
}

impl Overlay {
    /// Returns whether Escape can close it. A toast leaves on its own.
    pub(crate) fn dismissible(&self) -> bool {
        self.layer != Layer::Toast
    }

    /// Returns whether a press outside it or the window losing the input
    /// focus closes it: a popup or a tooltip does, while a dialog, which
    /// often holds work in progress, and a toast stay.
    pub(crate) fn transient(&self) -> bool {
        matches!(self.layer, Layer::Popup | Layer::Tooltip)
    }
}

impl WidgetTree {
    /// Returns whether `id` is the top node of an overlay.
    pub(crate) fn is_overlay_root(&self, id: WidgetId) -> bool {
        self.overlays.iter().any(|overlay| overlay.root == id)
    }

    /// Returns the parent of `id` as the pointer sees it: none for the top
    /// node of an overlay, whose host lies under it in the window but not
    /// under it on the screen.
    pub(crate) fn pointer_parent(&self, id: WidgetId) -> Option<WidgetId> {
        if self.is_overlay_root(id) {
            None
        } else {
            self.nodes.get(id)?.parent
        }
    }

    /// Returns whether the overlay `overlay` shows: its host and every
    /// ancestor of the host are live and neither hidden nor fading out.
    pub(crate) fn overlay_shown(&self, overlay: &Overlay) -> bool {
        let mut next = self.nodes.get(overlay.root).and_then(|node| node.parent);
        while let Some(id) = next {
            let Some(node) = self.nodes.get(id) else {
                return false;
            };
            if node.withdrawn() {
                return false;
            }
            next = node.parent;
        }
        true
    }

    /// Returns the deepest node `position` hits, trying the overlays first,
    /// the topmost first, then the main tree. A point inside an overlay's
    /// rectangle stays there even where nothing in the overlay is hit, so the
    /// main tree under it hears nothing. A toast overlay holds only the
    /// points its toasts cover: its own rectangle, which spans the gaps
    /// between them and the room beside a narrow one, lets the rest through.
    pub(crate) fn hit_any(&self, position: Point, disabled: bool) -> Option<WidgetId> {
        for overlay in self.overlays.iter().rev() {
            let fading = self
                .nodes
                .get(overlay.root)
                .is_none_or(|node| node.withdrawn());
            if overlay.passes_pointer || fading || !self.overlay_shown(overlay) {
                continue;
            }
            if overlay.layer == Layer::Toast {
                if let Some(hit) = self
                    .hit(overlay.root, position, disabled)
                    .filter(|&hit| hit != overlay.root)
                {
                    return Some(hit);
                }
                continue;
            }
            if let Some(hit) = self.hit(overlay.root, position, disabled) {
                return Some(hit);
            }
            if self
                .nodes
                .get(overlay.root)
                .is_some_and(|node| node.rect.contains(position))
            {
                return None;
            }
        }
        self.root
            .and_then(|root| self.hit(root, position, disabled))
    }

    /// Places every overlay against its host, and lays out the subtree of
    /// each one whose place changed, or of every one with `all`. Returns
    /// whether any rectangle or clip changed.
    pub(crate) fn place_overlays(&mut self, all: bool) -> bool {
        let Some(viewport) = self.viewport else {
            return false;
        };
        let window = Rect::new(0.0, 0.0, viewport.width, viewport.height);
        let mut changed = false;
        for at in 0..self.overlays.len() {
            let (root, placement) = (self.overlays[at].root, self.overlays[at].placement);
            let Some(host) = self.nodes.get(root).and_then(|node| node.parent) else {
                continue;
            };
            let anchor = self
                .nodes
                .get(host)
                .map(|node| node.rect)
                .unwrap_or_default();
            // The layout root's location already holds its margin, so the
            // overlay is placed as its margin box.
            let size = self
                .taffy
                .layout(self.nodes[root].taffy)
                .map(|layout| Size {
                    width: finite_length(
                        layout.size.width + layout.margin.left + layout.margin.right,
                    ),
                    height: finite_length(
                        layout.size.height + layout.margin.top + layout.margin.bottom,
                    ),
                })
                .unwrap_or_default();
            let origin = placement.place(anchor, size, viewport);
            if !all && origin == self.overlays[at].origin {
                continue;
            }
            self.overlays[at].origin = origin;
            let hidden = !self.overlay_shown(&self.overlays[at]);
            changed |= self.update_overlay(root, origin, window, hidden);
        }
        changed
    }

    /// Places the overlay whose top node is `root` by `placement` from now
    /// on, and moves it at once when that changes where it goes.
    pub(crate) fn set_overlay_placement(&mut self, root: WidgetId, placement: Placement) {
        let Some(overlay) = self
            .overlays
            .iter_mut()
            .find(|overlay| overlay.root == root)
        else {
            return;
        };
        if overlay.placement != placement {
            overlay.placement = placement;
            // Covering the window, or no longer covering it, changes the
            // size the root resolves to.
            self.re_resolve(root);
            self.place_overlays(false);
        }
    }

    /// Closes the popups and tooltips a press at `position` lands outside
    /// of, the topmost first: those whose subtree the press misses, unless it
    /// lands on the node one hangs off.
    pub(crate) fn dismiss_outside(&mut self, position: Point) {
        let hit = self.hit_any(position, true);
        let outside: Vec<RwSignal<bool>> = self
            .overlays
            .iter()
            .rev()
            .filter(|overlay| overlay.transient())
            .filter(|overlay| {
                let inside = hit.is_some_and(|hit| self.is_within(hit, overlay.root));
                let on_anchor = overlay.placement.anchored_to_node()
                    && hit.is_some_and(|hit| {
                        self.nodes
                            .get(overlay.root)
                            .and_then(|node| node.parent)
                            .is_some_and(|host| self.is_within(hit, host))
                    });
                !inside && !on_anchor
            })
            .map(|overlay| overlay.open.clone())
            .collect();
        for open in outside {
            if open.get() {
                open.set(false);
            }
        }
    }

    /// Closes the last opened overlay that shows when `event` is a fresh
    /// press of Escape, and returns whether one closed.
    pub(crate) fn dismiss_on_escape(&mut self, event: &KeyEvent) -> bool {
        if !event.pressed || event.key != Key::Escape {
            return false;
        }
        let Some(open) = self
            .overlays
            .iter()
            .rev()
            .find(|overlay| {
                overlay.dismissible() && self.overlay_shown(overlay) && overlay.open.get()
            })
            .map(|overlay| overlay.open.clone())
        else {
            return false;
        };
        open.set(false);
        true
    }
}

impl WidgetTree {
    /// Returns where an overlay of `layer` that `host` opens sorts among
    /// the overlays, and counts it as opened.
    pub(crate) fn overlay_order(&mut self, host: WidgetId, layer: Layer) -> (u64, u8, u64) {
        self.overlay_count += 1;
        let opened = self.overlay_count;
        let dialog = || {
            let mut next = Some(host);
            while let Some(id) = next {
                if let Some(overlay) = self
                    .overlays
                    .iter()
                    .find(|overlay| overlay.root == id && overlay.layer == Layer::Dialog)
                {
                    return overlay.order.0;
                }
                next = self.nodes.get(id).and_then(|node| node.parent);
            }
            0
        };
        match layer {
            Layer::Dialog => (opened, 0, opened),
            Layer::Tooltip => (dialog(), 2, opened),
            Layer::Toast => (u64::MAX, 0, opened),
            _ => (dialog(), 1, opened),
        }
    }

    /// Closes the popups and tooltips, which the window losing the input
    /// focus does. Dialogs and toasts stay.
    pub(crate) fn close_transient_overlays(&mut self) {
        let transient: Vec<RwSignal<bool>> = self
            .overlays
            .iter()
            .filter(|overlay| overlay.transient())
            .map(|overlay| overlay.open.clone())
            .collect();
        for open in transient {
            if open.get() {
                open.set(false);
            }
        }
    }
}

/// Returns `length`, or zero when it is not finite and positive.
fn finite_length(length: f32) -> f32 {
    if length.is_finite() && length > 0.0 {
        length
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod tests;
