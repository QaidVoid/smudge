//! Every public constructor passes its caller on, so the devtools name the
//! app's line that built a node, not one inside the library.

use std::rc::Rc;

use smudge_core::element::{IntoElement, for_each, show};
use smudge_core::reactive::RwSignal;
use smudge_core::style::Direction;
use smudge_core::widget::panel;
use smudge_test::Harness;
use smudge_text::{TextSystem, label, rich_text, selection_scope, text_area, text_input};
use smudge_widgets::{
    button, column, image, row, scroll, spacer, split, stack, virtual_column_keyed, virtual_list,
    virtual_list_keyed,
};

/// Returns the file and line the root node of `build` was created at.
fn site<E: IntoElement>(build: impl FnOnce() -> E + 'static) -> (&'static str, u32) {
    TextSystem::bundled_only().install();
    let mut harness = Harness::new(200.0, 100.0, 1.0, build);
    harness.frame();
    let mut root = None;
    harness.app().visit_tree(|entry| {
        root.get_or_insert(entry.id);
    });
    let at = harness
        .app()
        .node_details(root.expect("the build mounted a root"))
        .expect("the root is live")
        .created_at
        .expect("an element built the root");
    (at.file(), at.line())
}

// A function item handed over as the build is called from inside the
// standard library, which then is its caller, so each build is a closure.
#[allow(clippy::redundant_closure)]
#[test]
fn every_public_constructor_names_the_apps_line() {
    let line = line!() + 2;
    let sites = [
        ("panel", site(|| panel())),
        ("row", site(|| row())),
        ("column", site(|| column())),
        ("stack", site(|| stack())),
        ("spacer", site(|| spacer())),
        ("button", site(|| button("Save"))),
        ("label", site(|| label("Hello"))),
        ("rich_text", site(|| rich_text(Rc::<[_]>::from([])))),
        (
            "text_input",
            site(|| text_input(RwSignal::new(String::new()))),
        ),
        (
            "text_area",
            site(|| text_area(RwSignal::new(String::new()))),
        ),
        ("image", site(|| image(None))),
        ("scroll", site(|| scroll(panel()))),
        ("split", site(|| split(Direction::Row, panel(), panel()))),
        (
            "virtual_list",
            site(|| virtual_list(|| 3, 20.0, |_| panel())),
        ),
        (
            "virtual_list_keyed",
            site(|| virtual_list_keyed(|| Rc::<[u32]>::from([1, 2]), |n| *n, 20.0, |_| panel())),
        ),
        (
            "virtual_column_keyed",
            site(|| virtual_column_keyed(|| Rc::<[u32]>::from([1, 2]), |n| *n, 20.0, |_| panel())),
        ),
        ("show", site(|| show(|| true, panel))),
        (
            "for_each",
            site(|| for_each(|| vec![1_u32], |n| *n, |_| panel())),
        ),
        (
            "selection_scope",
            site(|| selection_scope(|| Rc::from([1_u32]), |_: &u32| String::new(), panel)),
        ),
    ];
    let wrong: Vec<_> = sites
        .iter()
        .filter(|(_, (file, _))| *file != file!())
        .collect();
    assert!(
        wrong.is_empty(),
        "these name a line inside the library: {wrong:?}"
    );
    assert_eq!(sites[0].1, (file!(), line), "the line of the call");
}
