use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use crate::reactive::{Owner, runtime::Scope};
use crate::tree::WidgetId;

use super::{Update, UpdateQueue};

/// A focus destination that follows an element across mounts.
///
/// Attach it with [`Element::focus_target`](super::Element::focus_target).
/// Use a separate target for each destination. A new mount of the same
/// target takes over from its previous mount.
///
/// ```
/// use smudge_core::element::FocusTarget;
/// use smudge_core::widget::panel;
///
/// let target = FocusTarget::default();
/// let destination = panel().focus_target(&target).focusable();
/// let jump = panel().on_press(move || target.request_focus(None));
/// let view = panel().child(destination).child(jump);
/// ```
#[derive(Clone, Default)]
pub struct FocusTarget(Rc<RefCell<Option<Mounted>>>);

struct Mounted {
    id: WidgetId,
    queue: Weak<RefCell<VecDeque<Update>>>,
    scope: Weak<Scope>,
}

impl FocusTarget {
    /// Requests focus on the next frame, after mounts and overlay changes.
    ///
    /// Tries the last focused node in the destination's subtree, then the
    /// destination or its first Tab stop. If none can take focus, tries
    /// `fallback` in the same app. Hidden and disabled nodes and nodes
    /// outside the active focus trap cannot take focus. An unattached target
    /// can only request focus through an attached fallback. When multiple
    /// requests are made before a frame, only the latest one is resolved.
    pub fn request_focus(&self, fallback: Option<&Self>) {
        let mounted_app = |target: &Self| {
            let mounted = target.0.borrow();
            let mounted = mounted.as_ref()?;
            Some((mounted.queue.upgrade()?, mounted.scope.upgrade()?))
        };
        let Some((queue, scope)) = mounted_app(self).or_else(|| fallback.and_then(mounted_app))
        else {
            return;
        };
        let (primary, fallback) = (self.clone(), fallback.cloned());
        UpdateQueue(queue).push(Box::new(move |tree| {
            tree.pending_focus_target = Some((primary, fallback));
        }));
        if !scope.quiet.get() {
            let wake = scope.wake.borrow().clone();
            if let Some(wake) = wake {
                wake();
            }
        }
    }

    pub(crate) fn mount(&self, id: WidgetId, queue: &UpdateQueue, owner: &Owner) {
        *self.0.borrow_mut() = Some(Mounted {
            id,
            queue: Rc::downgrade(&queue.0),
            scope: Rc::downgrade(&owner.data().scope),
        });
        let target = self.clone();
        let queue_id = Rc::downgrade(&queue.0);
        Owner::on_cleanup(move || {
            let mut mounted = target.0.borrow_mut();
            if mounted
                .as_ref()
                .is_some_and(|mounted| mounted.id == id && mounted.queue.ptr_eq(&queue_id))
            {
                *mounted = None;
            }
        });
    }

    pub(crate) fn mounted(&self) -> Option<(WidgetId, Rc<Scope>)> {
        let mounted = self.0.borrow();
        let mounted = mounted.as_ref()?;
        Some((mounted.id, mounted.scope.upgrade()?))
    }
}
