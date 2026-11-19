//! Whole-frame behavior of a markdown view over a growing source, driven
//! through the headless harness with the bundled fonts.

use smudge_core::clipboard::{Clipboard, MemoryClipboard};
use smudge_core::geometry::Point;
use smudge_core::input::{Key, Modifiers};
use smudge_core::reactive::Owner;
use smudge_core::style::{pct, px};
use smudge_markdown::{MarkdownOptions, MarkdownSource, markdown};
use smudge_test::Harness;
use smudge_text::TextSystem;

/// The window the view fills.
const WINDOW: (f32, f32) = (400.0, 600.0);

/// A harness showing `source` in a view that fills the window, with a
/// memory clipboard installed.
fn view_of(source: &MarkdownSource) -> (Harness, MemoryClipboard) {
    TextSystem::bundled_only().install();
    let shown = source.clone();
    let mut harness = Harness::new(WINDOW.0, WINDOW.1, 1.0, move || {
        markdown(&shown, MarkdownOptions::default())
            .width(pct(100.0))
            .height(px(WINDOW.1))
    });
    let clipboard = MemoryClipboard::default();
    harness.app().set_clipboard(Box::new(clipboard.clone()));
    harness.frame();
    harness.settle();
    (harness, clipboard)
}

/// The vertical extent of every paragraph the view shows, top to bottom.
fn paragraphs(harness: &Harness) -> Vec<(f32, f32)> {
    harness
        .layout_dump()
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix("RichText [")?;
            let values: Vec<f32> = rest
                .trim_end_matches(']')
                .split(", ")
                .map(|value| value.parse().expect("numbers"))
                .collect();
            Some((values[1], values[3]))
        })
        .collect()
}

/// The modifiers of the copy and select-all shortcuts on this platform.
fn command() -> Modifiers {
    Modifiers {
        control: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Modifiers::default()
    }
}

/// Selects everything the view shows and copies it.
fn copy_all(harness: &mut Harness, clipboard: &mut MemoryClipboard) -> Option<String> {
    let (top, bottom) = paragraphs(harness)[0];
    harness.click(Point {
        x: 10.0,
        y: (top + bottom) / 2.0,
    });
    harness.frame();
    harness.press_key_with(Key::Character(String::from("a")), command());
    harness.frame();
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    clipboard.get()
}

#[test]
fn an_append_repaints_the_open_block_alone() {
    let source = MarkdownSource::new();
    source.push("The first paragraph.\n\nThe second one.\n\nThe third");
    let (mut harness, _) = view_of(&source);
    let shown = paragraphs(&harness);
    assert_eq!(shown.len(), 3, "three paragraphs show: {shown:?}");
    let open_top = shown[2].0;

    source.push(" keeps growing");
    let report = harness.frame();
    assert!(!report.full, "the append repaints part of the window");
    assert!(!report.damage.is_empty(), "the open paragraph repaints");
    for rect in &report.damage {
        assert!(
            rect.y as f32 >= open_top - 1.0,
            "the damage {rect:?} reaches above the open block at {open_top}"
        );
    }
    harness.assert_matches_full_repaint();
}

#[test]
fn a_closed_block_keeps_its_selection_while_the_document_grows() {
    let source = MarkdownSource::new();
    source.push("Kept text.\n\nGrowing");
    let (mut harness, mut clipboard) = view_of(&source);
    let (top, bottom) = paragraphs(&harness)[0];
    let middle = (top + bottom) / 2.0;
    harness.button_down(Point { x: 2.0, y: middle });
    harness.move_to(Point {
        x: 300.0,
        y: middle,
    });
    harness.button_up(Point {
        x: 300.0,
        y: middle,
    });
    harness.frame();

    for word in [" one", " two", " three\n\nA new block"] {
        source.push(word);
        harness.frame();
    }
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    assert_eq!(
        clipboard.get().as_deref(),
        Some("Kept text."),
        "the closed block kept its selection while the document grew"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_reference_resolves_when_its_definition_arrives_and_only_its_block_repaints() {
    let source = MarkdownSource::new();
    source.push("See [x] here.\n\nb\n\nc\n\nd\n\n");
    let (mut harness, mut clipboard) = view_of(&source);
    assert_eq!(
        copy_all(&mut harness, &mut clipboard).as_deref(),
        Some("See [x] here.\nb\nc\nd"),
        "with no definition, the reference shows its source"
    );
    let unchanged = paragraphs(&harness)[1..4].to_vec();

    source.push("[x]: /target\n\ne\n");
    let report = harness.frame();
    for rect in &report.damage {
        let (top, bottom) = (rect.y as f32, (rect.y + rect.height) as f32);
        for (block_top, block_bottom) in &unchanged {
            assert!(
                bottom <= *block_top + 1.0 || top >= *block_bottom - 1.0,
                "the damage {rect:?} repainted a block between {block_top} and {block_bottom}"
            );
        }
    }
    harness.assert_matches_full_repaint();
    assert_eq!(
        copy_all(&mut harness, &mut clipboard).as_deref(),
        Some("See x here.\nb\nc\nd\ne"),
        "the reference became a link to its definition"
    );
}

#[test]
fn a_long_transcript_mounts_only_the_blocks_near_the_viewport_and_follows_its_end() {
    let source = MarkdownSource::new();
    let text: String = (0..1_000)
        .map(|block| format!("Paragraph number {block}.\n\n"))
        .collect();
    source.push(&text);
    TextSystem::bundled_only().install();
    let shown = source.clone();
    let mut harness = Harness::new(WINDOW.0, WINDOW.1, 1.0, move || {
        markdown(
            &shown,
            MarkdownOptions::default().follow_end(smudge_core::reactive::RwSignal::new(true)),
        )
        .width(pct(100.0))
        .height(px(WINDOW.1))
    });
    harness.frame();
    harness.settle();
    let mounted = paragraphs(&harness).len();
    assert!(
        mounted < 100,
        "a 1,000 block transcript mounted {mounted} paragraphs"
    );

    source.push("The streamed end");
    let report = harness.frame();
    assert!(
        !report.damage.is_empty(),
        "the end the view follows repaints as it grows"
    );
}

#[test]
fn a_cleared_source_shows_the_next_document_and_not_the_last() {
    let source = MarkdownSource::new();
    source.push("First reply.\n\nIts second block.");
    let (mut harness, mut clipboard) = view_of(&source);
    source.clear();
    source.push("Second reply.");
    harness.frame();
    harness.settle();
    assert_eq!(
        copy_all(&mut harness, &mut clipboard).as_deref(),
        Some("Second reply."),
        "every row shows the new document"
    );
    source.push(" Grows.");
    harness.frame();
    assert_eq!(
        copy_all(&mut harness, &mut clipboard).as_deref(),
        Some("Second reply. Grows."),
        "the open block's row follows the new document's signal"
    );
    harness.assert_matches_full_repaint();
}

#[test]
fn a_block_opened_under_an_owner_that_ends_keeps_repainting_as_it_grows() {
    let source = MarkdownSource::new();
    source.push("Kept.\n\n");
    let (mut harness, _) = view_of(&source);
    let passing = Owner::new();
    passing.with(|| source.push("Opened under a passing owner.\n\nNext"));
    passing.cleanup();
    drop(passing);
    harness.frame();
    harness.settle();
    let shown = paragraphs(&harness);
    assert_eq!(shown.len(), 3, "three paragraphs show: {shown:?}");
    source.push(" grows");
    let report = harness.frame();
    assert!(
        !report.damage.is_empty(),
        "the block opened under the passing owner repaints as it grows"
    );
    harness.assert_matches_full_repaint();
}

/// The rectangle of the first node of the dump whose line starts with
/// `prefix`, as x, y, width and height.
fn rect_of(harness: &Harness, prefix: &str) -> Vec<f32> {
    let dump = harness.layout_dump();
    let line = dump
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with(prefix))
        .expect("the dump shows the node");
    line[line.find('[').expect("a rect") + 1..line.find(']').expect("a rect")]
        .split(", ")
        .map(|value| value.parse().expect("numbers"))
        .collect()
}

#[test]
fn a_long_list_number_gets_the_width_it_needs_beside_its_item_text() {
    TextSystem::bundled_only().install();
    let mut alone = Harness::new(WINDOW.0, WINDOW.1, 1.0, || {
        smudge_core::widget::panel().child(smudge_text::label("123456789."))
    });
    alone.frame();
    let needed = rect_of(&alone, "Label [")[2];

    let source = MarkdownSource::new();
    source.push("123456789. item\n");
    let (harness, _) = view_of(&source);
    let marker = rect_of(&harness, "Label [");
    let item = rect_of(&harness, "RichText [");
    assert!(
        marker[2] >= needed,
        "the marker {marker:?} is narrower than its text, {needed}"
    );
    assert!(
        marker[0] + marker[2] <= item[0],
        "the marker {marker:?} ends before the item {item:?} starts"
    );
}

#[test]
fn a_selection_in_the_streaming_paragraph_survives_its_line_ending() {
    let source = MarkdownSource::new();
    source.push("First.\n\nSecond");
    let (mut harness, mut clipboard) = view_of(&source);
    let (top, bottom) = paragraphs(&harness)[1];
    let middle = (top + bottom) / 2.0;
    harness.button_down(Point { x: 1.0, y: middle });
    harness.move_to(Point {
        x: 300.0,
        y: middle,
    });
    harness.button_up(Point {
        x: 300.0,
        y: middle,
    });
    harness.frame();

    source.push("\n");
    harness.frame();
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    assert_eq!(
        clipboard.get().as_deref(),
        Some("Second"),
        "the paragraph moved into a block of its own and kept its selection"
    );
}

#[test]
fn a_list_that_replaces_another_shows_its_own_numbers() {
    let source = MarkdownSource::new();
    source.push("[a|\n|-\n3. old\n");
    let (mut harness, _) = view_of(&source);
    source.push("b]: /x\n\nNew paragraph\n\n5. new");
    harness.frame();
    harness.settle();

    let fresh = MarkdownSource::new();
    fresh.push(&source.text());
    let (expected, _) = view_of(&fresh);
    harness.assert_matches(&expected);
}

#[test]
fn a_selection_in_a_list_item_survives_the_list_turning_loose() {
    let source = MarkdownSource::new();
    source.push("- \\*one\n- two");
    let (mut harness, mut clipboard) = view_of(&source);
    let (top, bottom) = paragraphs(&harness)[0];
    let middle = (top + bottom) / 2.0;
    harness.button_down(Point { x: 30.0, y: middle });
    harness.move_to(Point {
        x: 390.0,
        y: middle,
    });
    harness.button_up(Point {
        x: 390.0,
        y: middle,
    });
    harness.frame();

    source.push("\n\n  more");
    harness.frame();
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    assert_eq!(clipboard.get().as_deref(), Some("*one"));
}

#[test]
fn a_selection_end_inside_a_resolved_grapheme_copies_the_whole_grapheme() {
    let source = MarkdownSource::new();
    source.push("[e\u{301}]\n\n");
    let (mut harness, mut clipboard) = view_of(&source);
    let (top, bottom) = paragraphs(&harness)[0];
    harness.click(Point {
        x: 390.0,
        y: (top + bottom) / 2.0,
    });
    harness.frame();
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for _ in 0..2 {
        harness.press_key_with(Key::ArrowLeft, shift);
        harness.frame();
    }

    source.push("[e\u{301}]: /target\n");
    harness.frame();
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    assert_eq!(clipboard.get().as_deref(), Some("e\u{301}"));
}

#[test]
fn a_caret_placed_before_a_block_opens_still_extends_with_the_keyboard() {
    let source = MarkdownSource::new();
    source.push("First.\n\nSecond");
    let (mut harness, mut clipboard) = view_of(&source);
    let (top, bottom) = paragraphs(&harness)[1];
    harness.click(Point {
        x: 390.0,
        y: (top + bottom) / 2.0,
    });
    harness.frame();

    source.push("\n");
    harness.frame();
    harness.press_key_with(
        Key::ArrowLeft,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    harness.frame();
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    assert_eq!(clipboard.get().as_deref(), Some("d"));
}

#[test]
fn an_unmounted_block_s_grapheme_copies_whole_after_its_reference_resolves() {
    let source = MarkdownSource::new();
    let rest: String = (0..200)
        .map(|block| format!("Paragraph {block}.\n\n"))
        .collect();
    source.push(&format!("[e\u{301}]\n\n{rest}"));
    let (mut harness, mut clipboard) = view_of(&source);
    let (top, bottom) = paragraphs(&harness)[0];
    harness.click(Point {
        x: 390.0,
        y: (top + bottom) / 2.0,
    });
    harness.frame();
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for _ in 0..2 {
        harness.press_key_with(Key::ArrowLeft, shift);
        harness.frame();
    }

    for _ in 0..20 {
        harness.event(smudge_core::input::InputEvent::Wheel {
            delta: Point { x: 0.0, y: 400.0 },
        });
        harness.frame();
    }
    harness.settle();
    let mounted = paragraphs(&harness).len();
    assert!(
        mounted < 100,
        "the view scrolled far enough to unmount most blocks: {mounted}"
    );
    source.push("[e\u{301}]: /target\n");
    harness.frame();
    harness.press_key_with(Key::Character(String::from("c")), command());
    harness.frame();
    assert_eq!(clipboard.get().as_deref(), Some("e\u{301}"));
}

#[test]
fn headings_reach_the_accessibility_tree_with_their_levels() {
    let source = MarkdownSource::new();
    source.push("# Title\n\nSome text.\n\n### Detail\n\nMore.\n");
    let (mut harness, _) = view_of(&source);
    let headings: Vec<(Option<String>, Option<usize>)> = harness
        .app()
        .semantics_nodes()
        .into_iter()
        .filter(|node| node.role == smudge_core::semantics::Role::Heading)
        .map(|node| (node.label, node.level))
        .collect();
    assert_eq!(
        headings,
        vec![
            (Some("Title".to_owned()), Some(1)),
            (Some("Detail".to_owned()), Some(3)),
        ],
        "each heading is named by its text and reports its level"
    );
}
