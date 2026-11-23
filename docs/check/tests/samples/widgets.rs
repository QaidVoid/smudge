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
        .padding_xy(Size { width: 8.0, height: 6.0 })
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
fn sideways() {
    use smudge::widgets::{ScrollAxis, ScrollExt};
    
    let across = RwSignal::new(0.0_f32);
    
    let timeline = scroll(row().children((0..48).map(|slot| label(format!("Slot {slot}")).width(px(80.0)))))
        .axis(ScrollAxis::Horizontal)
        .horizontal_offset(across.clone())
        .height(px(40.0));
}

#[test]
fn split_panes() {
    use smudge::prelude::*;
    
    let ratio = RwSignal::new(0.3);
    let editor = split(
        Direction::Row,
        label("Files"),
        split(Direction::Column, label("Editor"), label("Terminal")).min_second(80.0),
    )
    .ratio(ratio)
    .min_first(160.0)
    .min_second(240.0);
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

#[test]
fn canvas_sample() {
    use smudge::core::path::{Dash, LineCap};

    let selection = RwSignal::new(Rect::new(20.0, 16.0, 120.0, 72.0));
    let shown = selection.clone();
    let view = canvas(move |painter| {
        let size = painter.size();
        for x in (0..size.width as u32).step_by(16) {
            let x = x as f32;
            painter.stroke(
                &Path::line(Point { x, y: 0.0 }, Point { x, y: size.height }),
                Stroke::new(1.0, Color::rgb(60, 64, 72)),
            );
        }
        let area = shown.get();
        painter.fill_rect(area, Corners::ZERO, Color::rgba(80, 150, 255, 40));
        painter.stroke(
            &Path::polygon([
                Point { x: area.x0, y: area.y0 },
                Point { x: area.x1, y: area.y0 },
                Point { x: area.x1, y: area.y1 },
                Point { x: area.x0, y: area.y1 },
            ]),
            Stroke::new(1.5, Color::rgb(80, 150, 255))
                .with_cap(LineCap::Square)
                .with_dash(Dash::new([6.0, 4.0])),
        );
        painter.text("selection", Point { x: area.x0, y: area.y1 + 4.0 }, 12.0, Color::WHITE);
    })
    .height(px(240.0));
}

#[test]
fn overlay_sample() {
    use smudge::core::element::Placement;

    let open = RwSignal::new(false);
    let toggle = open.clone();
    let picker = button("Colors")
        .on_press(move || toggle.update(|open| *open = !*open))
        .overlay(open, Placement::Below, || {
            column()
                .padding(px(8.0))
                .background(ColorToken::SURFACE)
                .border(1.0)
                .border_color(ColorToken::BORDER)
                .child(label("Red"))
                .child(label("Green"))
        });
}

#[test]
fn menus_sample() {
    use smudge::widgets::{
        ContextMenuExt, menu_bar, menu_check_item, menu_item, menu_separator, submenu,
    };

    let grid = RwSignal::new(true);
    let shown = grid.clone();
    let bar = menu_bar()
        .menu("File", || {
            vec![
                menu_item("Open", || println!("open")).shortcut("Ctrl+O"),
                submenu("Recent", || vec![menu_item("map.map", || {})]),
                menu_separator(),
                menu_item("Quit", || {}).disabled(true),
            ]
        })
        .menu("View", move || vec![menu_check_item("Grid", shown.clone())]);

    let canvas_area = panel()
        .grow(1.0)
        .focusable()
        .context_menu(|| vec![menu_item("Paste here", || {})]);
}

#[test]
fn dropdown_sample() {
    use smudge::widgets::{TooltipExt, dropdown};

    let brush = RwSignal::new(1_usize);
    let picker = row()
        .child(
            dropdown(
                || vec![String::from("Small"), String::from("Medium"), String::from("Large")],
                brush,
            )
            .accessible_label("Brush size"),
        )
        .child(button("Clear").tooltip_text("Clear the canvas"));
}

#[test]
fn dialog_sample() {
    use smudge::widgets::modal;
    
    let open = RwSignal::new(false);
    let (ask, close) = (open.clone(), open.clone());
    
    let page = column()
        .child(button("Delete").on_press(move || ask.set(true)))
        .child(modal(open, move || {
            let close = close.clone();
            column()
                .padding(px(16.0))
                .background(ColorToken::SURFACE)
                .accessible_label("Delete the map?")
                .child(label("Delete the map?"))
                .child(button("Cancel").on_press(move || close.set(false)))
        }));
}

#[test]
fn toast_sample() {
    use smudge::widgets::{Toasts, toast_host};
    
    let toasts = Toasts::new();
    let saved = toasts.clone();
    
    let page = column()
        .child(button("Save").on_press(move || saved.show("Saved")))
        .child(toast_host(&toasts));
}

#[test]
fn separator_sample() {
    use smudge::widgets::separator;

    let form = column()
        .gap(Size { width: 0.0, height: 8.0 })
        .child(label("Account"))
        .child(separator())
        .child(label("Appearance"));
}

#[test]
fn hyperlink_sample() {
    use smudge::widgets::hyperlink;

    const GUIDE: &str = "https://smudge.example/guide";

    let link = hyperlink("Read the guide", || {
        // Opening a URL is a service of the platform, which phones get
        // through the platform layer. On the desktop, the app asks the
        // system's opener itself:
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(GUIDE).spawn();
    });
}

#[test]
fn color_picker_sample() {
    use smudge::widgets::{color_picker, slider};

    let brush = RwSignal::new(Color::rgb(230, 150, 60));
    let opacity = RwSignal::new(0.8_f64);
    let side = column()
        .gap(Size { width: 0.0, height: 10.0 })
        .child(color_picker(brush).accessible_label("Brush color"))
        .child(slider(opacity, 0.0..=1.0, 0.05).accessible_label("Opacity"));
}

#[test]
fn controls_sample() {
    use smudge::widgets::{checkbox, number_field, slider, switch};

    let grid = RwSignal::new(true);
    let snap = RwSignal::new(false);
    let opacity = RwSignal::new(0.8);
    let width = RwSignal::new(12.0);
    let form = column()
        .child(checkbox("Show the grid", grid))
        .child(switch("Snap to grid", snap))
        .child(slider(opacity, 0.0..=1.0, 0.05).accessible_label("Opacity"))
        .child(number_field("Width", width, 1.0..=512.0, 1.0));
}

#[test]
fn tabs_sample() {
    use smudge::widgets::tabs;

    let shown = RwSignal::new(0_usize);
    let side = tabs(shown)
        .tab("Entities", || label("Every entity of the map"))
        .tab("Textures", || label("The texture browser"));
}

fn stat(title: &'static str, value: RwSignal<u32>) -> Element<Panel> {
    column()
        .padding(px(12.0))
        .corner_radius(RadiusToken::MEDIUM)
        .background(ColorToken::SURFACE)
        .child(label(title).text_color(ColorToken::TEXT_MUTED))
        .child(label(move || value.get().to_string()).text_size(24.0))
}
