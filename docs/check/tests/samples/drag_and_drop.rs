//! Mirrors the Rust samples in docs/guide/drag-and-drop.md.

#[test]
fn desktop_file_drop_zone() {
    use std::path::PathBuf;

    use smudge::core::theme::ColorValue;
    use smudge::prelude::*;

    let paths = RwSignal::new(Vec::<PathBuf>::new());
    let hovering = RwSignal::new(false);
    let highlight = hovering.clone();
    let dropped = paths.clone();

    let zone = panel()
        .size(Size { width: 280.0, height: 96.0 })
        .padding(px(16.0))
        .border(2.0)
        .border_color(move || {
            if highlight.get() {
                ColorValue::from(ColorToken::ACCENT)
            } else {
                ColorValue::from(ColorToken::BORDER)
            }
        })
        .on_file_hover(move |files| hovering.set(files.is_some()))
        .on_file_drop(move |files, _at| dropped.set(files.to_vec()))
        .child(label("Drop files here"));

    drop(zone);
}

#[test]
fn typed_in_app_drag() {
    use smudge::prelude::*;

    let received = RwSignal::new(String::new());
    let shown = received.clone();
    let source = panel()
        .padding(px(12.0))
        .background(ColorToken::SURFACE_HOVER)
        .child(label("report.pdf"))
        .drag_source(|| String::from("report.pdf"));
    let folder = panel()
        .min_width(px(160.0))
        .padding(px(12.0))
        .border(2.0)
        .border_color(ColorToken::BORDER)
        .drop_style(|style| style.border_color(ColorToken::ACCENT))
        .drop_target(
            |name: &String| name.ends_with(".pdf"),
            move |name, _at| received.set(name),
        )
        .child(label(move || format!("Folder: {}", shown.get())));
    let view = row().child(source).child(folder);

    drop(view);
}

#[test]
fn reorderable_playlist() {
    use smudge::prelude::*;
    use smudge::widgets::reorderable_list;

    fn main() -> Result<(), PlatformError> {
        let tracks = RwSignal::new(vec![(1, "Intro"), (2, "Main theme"), (3, "Outro")]);

        run(WindowOptions::default(), move || {
            column().padding(px(16.0)).child(
                reorderable_list(tracks.clone(), |track| track.0, 40.0, |track| {
                    panel()
                        .height(px(40.0))
                        .padding(px(8.0))
                        .child(label(track.1))
                })
                .height(px(160.0)),
            )
        })
    }

    let _: fn() -> Result<(), PlatformError> = main;
}
