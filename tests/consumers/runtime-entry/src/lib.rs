//! Generic reactive and text code instantiated in an application crate.
//!
//! Every value type here is local, so each generic function of the core and
//! the text crate that this code calls is compiled in this crate, the way it
//! is in an application.

use std::cell::Cell;
use std::rc::Rc;

use smudge_core::reactive::{
    self, Memo, Owner, RenderEffect, RwSignal, prelude::*, provide_context, signal_local,
    use_context,
};
use smudge_text::TextSystem;

/// A value type only this crate knows.
#[derive(Clone, PartialEq)]
pub struct Local(pub u32);

/// Creates, reads, writes and tracks signals, memos, effects, owners and
/// contexts of a local type, and returns a sum of what they saw.
pub fn exercise() -> u32 {
    let owner = Owner::new();
    let cleaned = Rc::new(Cell::new(0));
    let total = owner.with(|| {
        provide_context(Local(7));
        let signal = RwSignal::new(Local(1));
        let read = signal.clone();
        let doubled = Memo::new(move || Local(read.get().0 * 2));
        let seen = RwSignal::new(0_u32);
        let write = seen.clone();
        let _effect = RenderEffect::new(move || write.set(doubled.get().0));
        let (split, set_split) = signal_local(Local(3));
        signal.update(|value| value.0 += 1);
        signal.set(Local(5));
        set_split.set(Local(4));
        let marker = Rc::clone(&cleaned);
        owner
            .child()
            .with(|| Owner::on_cleanup(move || marker.set(1)));
        reactive::flush();
        let context = use_context::<Local>().map_or(0, |local| local.0);
        seen.get() + context + signal.with(|value| value.0) + split.get().0
    });
    owner.cleanup();
    total + cleaned.get()
}

/// Installs a text system and reaches it through its generic entry point,
/// with a closure and a result type only this crate knows.
pub fn text_system() -> u32 {
    TextSystem::bundled_only().install();
    let seen = Local(2);
    TextSystem::with(move |_system| Local(seen.0 + 1)).0
}

/// Wraps a closure of a local type the way a binding stores one, and runs
/// it through the wrapper, so the shim and the lookup are instantiated in
/// this crate.
pub fn hot_closure() -> u32 {
    let owner = Owner::new();
    let seen = owner.with(|| {
        let signal = RwSignal::new(Local(3));
        let read = signal.clone();
        let get = smudge_core::hot::wrap(move || Local(read.get().0 + 1));
        get().0
    });
    owner.cleanup();
    seen
}
