//! An editor's shell: a menu bar, a canvas with a context menu, and side
//! tabs of controls, tooltips and a dropdown.
//!
//! Run with `cargo run --example editor -p smudge`, then:
//!
//! - open the menus with the pointer or the keyboard, check View > Grid,
//!   and follow the submenu under File > Recent;
//! - right-click the canvas, or focus it and press the menu key or
//!   Shift+F10, for its context menu;
//! - pick a brush, its color, size and opacity in the side tabs, turn on
//!   snapping to the grid under Settings, and rest the pointer on the Clear
//!   button for its tooltip;
//! - with a screen reader, listen to the menus, the tabs and the controls.

use smudge::prelude::*;
use smudge::widgets::{
    ContextMenuExt, TooltipExt, checkbox, dropdown, menu_bar, menu_check_item, menu_item,
    menu_separator, number_field, slider, submenu, switch, tabs,
};

fn main() -> Result<(), PlatformError> {
    let grid = RwSignal::new(true);
    let snap = RwSignal::new(false);
    let size = RwSignal::new(8.0);
    let opacity = RwSignal::new(0.8_f64);
    let color = RwSignal::new(Color::rgb(230, 150, 60));
    let brush = RwSignal::new(0_usize);
    let tab = RwSignal::new(0_usize);
    let marks = RwSignal::new(Vec::<Point>::new());
    let status = RwSignal::new(String::from("Ready"));

    run_with(
        WindowOptions {
            title: String::from("Editor"),
            size: Size {
                width: 900.0,
                height: 600.0,
            },
            clear_color: Color::rgb(16, 17, 21),
            ..WindowOptions::default()
        },
        move || {
            let said = status.clone();
            let say = move |text: &'static str| {
                let said = said.clone();
                move || said.set(String::from(text))
            };
            let shown_grid = grid.clone();
            let bar = menu_bar()
                .menu("File", {
                    let say = say.clone();
                    move || {
                        vec![
                            menu_item("New", say("New map")).shortcut("Ctrl+N"),
                            menu_item("Open", say("Open a map")).shortcut("Ctrl+O"),
                            submenu("Recent", {
                                let say = say.clone();
                                move || {
                                    vec![
                                        menu_item("mp_harbor.map", say("Opened mp_harbor")),
                                        menu_item("mp_depot.map", say("Opened mp_depot")),
                                    ]
                                }
                            }),
                            menu_separator(),
                            menu_item("Quit", smudge::commands::close),
                        ]
                    }
                })
                .menu("View", move || {
                    vec![
                        menu_check_item("Grid", shown_grid.clone()),
                        menu_item("Zoom to fit", || {}).disabled(true),
                    ]
                });

            let (drawn, placed, cleared) = (marks.clone(), marks.clone(), marks.clone());
            let (grid_on, mark_size) = (grid.clone(), size.clone());
            let (shade, shape, snapping) = (opacity.clone(), brush.clone(), snap.clone());
            let tinted = color.clone();
            let canvas_view = canvas(move |painter| {
                let area = painter.size();
                if grid_on.get() {
                    let line = Stroke::new(1.0, Color::rgb(44, 47, 56));
                    let mut x = 0.0;
                    while x < area.width {
                        painter.stroke(
                            &Path::line(Point { x, y: 0.0 }, Point { x, y: area.height }),
                            line.clone(),
                        );
                        x += 32.0;
                    }
                    let mut y = 0.0;
                    while y < area.height {
                        painter.stroke(
                            &Path::line(Point { x: 0.0, y }, Point { x: area.width, y }),
                            line.clone(),
                        );
                        y += 32.0;
                    }
                }
                let radius = mark_size.get() as f32;
                let picked = tinted.get();
                let alpha = (shade.get().clamp(0.0, 1.0) * f64::from(picked.a)).round() as u8;
                let color = Color::rgba(picked.r, picked.g, picked.b, alpha);
                for at in drawn.get() {
                    match shape.get() {
                        1 => painter.fill_rect(
                            Rect::new(at.x - radius, at.y - radius, at.x + radius, at.y + radius),
                            Corners::ZERO,
                            color,
                        ),
                        2 => {
                            for (dx, dy) in [
                                (-0.6, -0.3),
                                (0.5, -0.6),
                                (0.1, 0.2),
                                (-0.4, 0.6),
                                (0.6, 0.5),
                            ] {
                                let dot = Point {
                                    x: at.x + dx * radius,
                                    y: at.y + dy * radius,
                                };
                                painter.fill(&Path::circle(dot, radius / 5.0), color);
                            }
                        }
                        _ => painter.fill(&Path::circle(at, radius), color),
                    }
                }
            })
            .grow(1.0)
            .focusable()
            .on_pointer_down(move |at| {
                let at = if snapping.get() {
                    let to_grid = |value: f32| (value / 32.0).round() * 32.0;
                    Point {
                        x: to_grid(at.x),
                        y: to_grid(at.y),
                    }
                } else {
                    at
                };
                placed.update(|marks| marks.push(at));
            })
            .context_menu(move || {
                let cleared = cleared.clone();
                vec![menu_item("Clear the marks", move || {
                    cleared.set(Vec::new())
                })]
            });

            let brushes = || ["Round", "Square", "Spray"].map(String::from).to_vec();
            let clear_marks = marks.clone();
            let side = tabs(tab.clone())
                .tab("Brush", {
                    let (size, opacity, brush, color) =
                        (size.clone(), opacity.clone(), brush.clone(), color.clone());
                    move || {
                        column()
                            .gap(Size {
                                width: 0.0,
                                height: 10.0,
                            })
                            .padding(px(12.0))
                            .child(dropdown(brushes, brush).accessible_label("Brush"))
                            .child(
                                row()
                                    .gap(Size {
                                        width: 8.0,
                                        height: 0.0,
                                    })
                                    .align_items(Align::Center)
                                    .child(color_picker(color).accessible_label("Brush color"))
                                    .child(label("Color")),
                            )
                            .child(slider(size, 2.0..=32.0, 1.0).accessible_label("Size"))
                            .child(slider(opacity, 0.0..=1.0, 0.05).accessible_label("Opacity"))
                            .child(
                                button("Clear")
                                    .on_press(move || clear_marks.set(Vec::new()))
                                    .tooltip_text("Clear every mark"),
                            )
                    }
                })
                .tab("Settings", {
                    let (grid, snap, size) = (grid.clone(), snap.clone(), size.clone());
                    move || {
                        column()
                            .gap(Size {
                                width: 0.0,
                                height: 10.0,
                            })
                            .padding(px(12.0))
                            .child(checkbox("Show the grid", grid))
                            .child(switch("Snap to grid", snap))
                            .child(number_field("Mark size", size, 2.0..=32.0, 1.0))
                    }
                });

            let shown_status = status.clone();
            column()
                .child(bar)
                .child(
                    row().grow(1.0).child(canvas_view).child(
                        column()
                            .width(px(260.0))
                            .border_edges(smudge::core::style::Edges {
                                top: 0.0,
                                right: 0.0,
                                bottom: 0.0,
                                left: 1.0,
                            })
                            .border_color(ColorToken::BORDER)
                            .child(side),
                    ),
                )
                .child(
                    label(move || shown_status.get())
                        .padding(px(6.0))
                        .text_color(ColorToken::TEXT_MUTED),
                )
        },
        |app| app.set_theme(Theme::dark()),
    )
}
