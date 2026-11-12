//! Mirrors docs/guide/widgets.md: its Rust samples, held as functions.
//!
//! Every ```rust block of the page appears here exactly as the page
//! shows it, reindented one level inside the wrapper each block needs,
//! and names the page leaves undefined are stubbed next to their
//! block. The page is the source of truth: change it here and there
//! together. The allows hold what page text may carry without a
//! compiler's blessing, so page edits trip this check only for real
//! API drift.
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_imports)]

use smudge::prelude::*;

#[test]
fn actions() {
    use smudge::prelude::*;
    
    let saves = RwSignal::new(0_u32);
    let save = saves.clone();
    
    let actions = row()
        .gap(Size { width: 8.0, height: 0.0 })
        .child(button("Cancel"))
        .child(button("Save").primary().on_press(move || save.update(|n| *n += 1)));
}

#[test]
fn labels() {
    let unread = RwSignal::new(0_u32);
    let title = label("Settings").text_size(22.0);
    let status = label(move || format!("{} unread", unread.get()));
}

#[test]
fn field() {
    let name = RwSignal::new(String::new());
    
    let field = text_input(name.clone())
        .width(pct(100.0))
        .height(px(32.0))
        .placeholder("Your name")
        .border(1.0)
        .border_color(ColorToken::BORDER);
}

#[test]
fn picture() {
    use smudge::core::image::Image;
    
    let source = RwSignal::new(None);
    let picture = image(move || source.get()).size(Size { width: 96.0, height: 96.0 });
}

#[test]
fn log() {
    let log = scroll(
        column().children((0..200).map(|line| label(format!("line {line}")))),
    )
    .height(px(300.0));
}

#[test]
fn offset() {
    let long_content = panel();
    use smudge::widgets::ScrollExt;
    
    let offset = RwSignal::new(0.0_f32);
    let to_top = offset.clone();
    
    let view = scroll(long_content)
        .offset(offset.clone())
        .height(px(300.0));
    let top = button("Back to top").on_press(move || to_top.set(0.0));
}

#[test]
fn follow() {
    let output_lines = panel();
    use smudge::widgets::ScrollExt;
    
    let follow = RwSignal::new(true);
    
    let log = scroll(output_lines).follow_end(follow).height(px(300.0));
}

#[test]
fn rows() {
    let rows = RwSignal::new(100_000_usize);
    let count = rows.clone();
    
    let list = virtual_list(
        move || count.get(),
        28.0,
        |index| label(format!("Row {index}")).height(px(28.0)).padding(px(4.0)),
    )
    .height(px(420.0));
}

#[test]
fn keyed() {
    use std::rc::Rc;
    
    use smudge::widgets::virtual_list_keyed;
    
    let items: RwSignal<Rc<[u32]>> = RwSignal::new(Rc::from([12, 40, 8, 24]));
    let source = items.clone();
    
    let list = virtual_list_keyed(
        move || source.get(),
        |item| *item,
        24.0,
        |item| label(format!("Item {item}")),
    )
    .height(px(420.0));
}

fn stat(title: &'static str, value: RwSignal<u32>) -> Element<Panel> {
    column()
        .padding(px(12.0))
        .corner_radius(RadiusToken::MEDIUM)
        .background(ColorToken::SURFACE)
        .child(label(title).text_color(ColorToken::TEXT_MUTED))
        .child(label(move || value.get().to_string()).text_size(24.0))
}
