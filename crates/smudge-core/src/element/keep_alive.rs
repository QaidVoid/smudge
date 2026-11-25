use std::rc::{Rc, Weak};

use crate::reactive::runtime::OwnerData;
use crate::tree::WidgetId;

use super::{
    AnyElement, Attach, Binding, Element, IntoElement, Owner, RenderEffect, Slot, UpdateQueue,
    mount_named,
};

type BuildPanel = dyn Fn(&str) -> AnyElement;

pub(crate) struct KeptContent {
    build: Box<BuildPanel>,
    owner: Option<Weak<OwnerData>>,
}

#[derive(Clone)]
pub(crate) struct KeptPlacement {
    store: Rc<KeptContent>,
    key: Rc<str>,
    queue: UpdateQueue,
}

impl KeptPlacement {
    pub(crate) fn place(&self, slot: WidgetId) {
        let placement = self.clone();
        self.queue.push(Box::new(move |tree| {
            if !tree.contains(slot) {
                return;
            }
            if !tree.kept_placement_can_claim(slot) {
                tree.keep_placement(slot, placement, true);
                return;
            }
            let store = &placement.store;
            let key = &placement.key;
            if let Some(child) = tree.kept_child(store, key) {
                tree.move_subtree(child, slot, 0);
            } else {
                let Some(root) = tree.root_owner() else {
                    return;
                };
                let anchor = store
                    .owner
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .filter(|owner| {
                        !owner.cleaned.get() && Rc::ptr_eq(&owner.scope, &root.data().scope)
                    });
                let anchor = anchor.as_ref().unwrap_or_else(|| root.data());
                #[cfg(all(feature = "hot-reload", target_os = "linux"))]
                let _mounting = crate::reactive::carry::Mounting::enter(anchor);
                let owner = anchor.child();
                let child = mount_named(
                    owner.with(|| (store.build)(key)),
                    tree,
                    &placement.queue,
                    Attach::Child(Some(slot)),
                    None,
                    Some(&owner),
                );
                tree.keep_child(Rc::clone(store), Rc::clone(key), child, owner);
            }
            tree.keep_placement(slot, placement, false);
        }));
    }
}

/// Shared keyed content that stays mounted when placed in a different slot.
///
/// Create the store in a scope that surrounds the placements. Each key's
/// builder runs under an owner in that scope, so state and context do not
/// depend on the placement. A store created outside the app uses the app's
/// root widget owner. Cloning the store shares its keys within the app.
///
/// ```
/// use smudge_core::element::{KeepAlive, show};
/// use smudge_core::reactive::{RwSignal, prelude::*};
/// use smudge_core::widget::panel;
///
/// let right = RwSignal::new(false);
/// let left = right.clone();
/// let panels = KeepAlive::new(|_| panel());
/// let other = panels.clone();
/// let root = panel()
///     .child(show(move || !left.get(), move || panels.slot("editor")))
///     .child(show(move || right.get(), move || other.slot("editor")));
/// ```
#[derive(Clone)]
pub struct KeepAlive(Rc<KeptContent>);

impl KeepAlive {
    /// Creates a store that builds the content for a key when it first mounts.
    pub fn new<E: IntoElement>(build: impl Fn(&str) -> E + 'static) -> Self {
        let build = crate::hot::wrap1_ref(build);
        Self(Rc::new(KeptContent {
            build: Box::new(move |key| build(key).into_any()),
            owner: Owner::current().map(|owner| Rc::downgrade(owner.data())),
        }))
    }

    /// Places `key` in this slot when it mounts or an exit fade around it is
    /// cancelled.
    /// If the key has another slot, this slot takes the content and the
    /// earlier one is empty.
    /// If no slot takes it by the end of the removal frame, it is unmounted
    /// and the next placement builds it again.
    ///
    /// The slot manages its own child. Set the placement's layout on the
    /// slot and keep panel state in the element the builder returns.
    pub fn slot(&self, key: impl Into<String>) -> Element<Slot> {
        let key: Rc<str> = Rc::from(key.into());
        let store = Rc::clone(&self.0);
        let mut element = Element::new(Slot);
        element.bindings.push(Binding {
            key: None,
            install: Box::new(move |slot, queue| {
                let placement = KeptPlacement {
                    store,
                    key,
                    queue: queue.clone(),
                };
                RenderEffect::new_unwrapped(move || placement.place(slot))
            }),
        });
        element
    }
}
