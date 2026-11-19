//! Hot reload carries the app's own signals across a remount, whatever
//! widgets with state of their own the patched build adds around them.

#![cfg(all(feature = "hot-reload", target_os = "linux"))]

use std::cell::RefCell;
use std::rc::Rc;

use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::style::Direction;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_widgets::{scroll, split};

#[test]
fn a_patch_that_adds_a_split_and_a_scroll_view_keeps_the_apps_signal() {
    let slot: Rc<RefCell<Option<RwSignal<f32>>>> = Rc::new(RefCell::new(None));
    let first = {
        let slot = Rc::clone(&slot);
        move || {
            *slot.borrow_mut() = Some(RwSignal::new(240.0_f32));
            panel()
        }
    };
    let patched = {
        let slot = Rc::clone(&slot);
        move || {
            let added = split(Direction::Row, panel(), scroll(panel()));
            *slot.borrow_mut() = Some(RwSignal::new(240.0_f32));
            panel().child(added)
        }
    };
    let mut harness = Harness::new(200.0, 100.0, 1.0, first);
    harness.frame();
    slot.borrow()
        .as_ref()
        .expect("the build made its signal")
        .set(99.0);

    // SAFETY: the test changes no type, so the layout it lists is the one
    // the values hold.
    unsafe {
        smudge_core::hot::note_kept_types(
            [smudge_core::hot::probe_of::<f32>()].into_iter().collect(),
        );
    }
    harness.app().remount(patched);
    let value = slot
        .borrow()
        .as_ref()
        .expect("the build made its signal")
        .get();
    assert_eq!(
        value, 99.0,
        "the widgets' own signals took none of the app's places"
    );
}
