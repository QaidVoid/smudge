//! Tests for the text runs a text widget reports.
//!
//! Every test installs a bundled-only system first, so no layout depends on
//! the fonts installed on the machine running the tests.

use smudge_core::geometry::Point;
use smudge_core::semantics::{TEXT_RUN_CAP, TextRun};

use crate::a11y::{Hidden, text_runs};
use crate::layout::TextLayout;
use crate::style::TextStyle;
use crate::system::TextSystem;

/// Lays `text` out at `width`, or on one line per hard break without one,
/// and returns its runs.
fn runs_of(text: &str, width: Option<f32>) -> Vec<TextRun> {
    TextSystem::bundled_only().install();
    let layout = TextLayout::new(text, &TextStyle::default(), width);
    text_runs(&layout.layout, text, Point::default(), None)
}

/// The words of `run`, as the word starts divide its text.
fn words(run: &TextRun) -> Vec<String> {
    let mut starts: Vec<usize> = run
        .word_starts
        .iter()
        .map(|start| usize::from(*start))
        .collect();
    starts.push(run.character_lengths.len());
    let offsets: Vec<usize> = std::iter::once(0)
        .chain(run.character_lengths.iter().scan(0, |at, len| {
            *at += usize::from(*len);
            Some(*at)
        }))
        .collect();
    starts
        .windows(2)
        .map(|pair| run.text[offsets[pair[0]]..offsets[pair[1]]].to_owned())
        .collect()
}

#[test]
fn the_runs_hold_the_text_a_wrapped_line_each() {
    let text = "several words that wrap onto more than one line";
    let runs = runs_of(text, Some(120.0));
    assert!(runs.len() > 1, "the text wraps: {runs:?}");
    let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(joined, text);
    for pair in runs.windows(2) {
        assert!(
            pair[1].bounds.y0 >= pair[0].bounds.y1 - 0.01,
            "each run sits below the one before"
        );
    }
    for run in &runs {
        let total: usize = run
            .character_lengths
            .iter()
            .map(|len| usize::from(*len))
            .sum();
        assert_eq!(total, run.text.len(), "the lengths count the whole run");
        assert_eq!(run.character_positions.len(), run.character_lengths.len());
        assert_eq!(run.character_widths.len(), run.character_lengths.len());
    }
}

#[test]
fn a_hard_break_ends_its_line_as_one_character() {
    let runs = runs_of("ab\ncd", None);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "ab\n");
    assert_eq!(runs[0].character_lengths, vec![1, 1, 1]);
    assert_eq!(runs[1].text, "cd");
}

#[test]
fn a_long_line_splits_into_runs_the_cap_can_count() {
    let text = "x".repeat(TEXT_RUN_CAP * 2 + 10);
    let runs = runs_of(&text, None);
    assert_eq!(runs.len(), 3);
    assert!(
        runs.iter()
            .all(|run| run.character_lengths.len() <= TEXT_RUN_CAP)
    );
    let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(joined, text);
    assert!(
        runs[1].bounds.x0 >= runs[0].bounds.x1 - 0.01,
        "the second stretch continues where the first ends"
    );
}

#[test]
fn a_letter_with_a_combining_mark_is_one_character() {
    let runs = runs_of("e\u{301}x", None);
    assert_eq!(runs[0].character_lengths, vec![3, 1]);
}

#[test]
fn a_cluster_longer_than_a_byte_counts_splits_into_pieces_that_fit() {
    let text = format!("a{}b", "\u{301}".repeat(200));
    let runs = runs_of(&text, None);
    let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(joined, text);
    let lengths = &runs[0].character_lengths;
    assert_eq!(
        lengths.first(),
        Some(&255),
        "the first piece is as long as fits"
    );
    assert!(
        runs[0].text[..255].chars().count() > 1,
        "the piece ends on a code point boundary"
    );
    assert_eq!(
        lengths.iter().map(|len| usize::from(*len)).sum::<usize>(),
        text.len()
    );
}

#[test]
fn a_word_takes_the_spaces_after_it() {
    let runs = runs_of("ab  cd e", None);
    assert_eq!(words(&runs[0]), vec!["ab  ", "cd ", "e"]);
}

#[test]
fn a_run_that_starts_inside_a_word_starts_no_word_there() {
    let text = format!("{} tail", "y".repeat(TEXT_RUN_CAP + 4));
    let runs = runs_of(&text, None);
    assert_eq!(runs.len(), 2);
    assert_eq!(
        words(&runs[1]),
        vec!["tail".to_owned()],
        "the stretch of the long word belongs to the word before"
    );
    assert_eq!(runs[1].word_starts, vec![5]);
}

#[test]
fn the_characters_sit_left_to_right_inside_the_run() {
    let runs = runs_of("abc def", None);
    let run = &runs[0];
    assert_eq!(run.character_positions[0], 0.0);
    for pair in run.character_positions.windows(2) {
        assert!(pair[1] > pair[0]);
    }
    let last = run.character_positions.len() - 1;
    let right = run.bounds.x0 + run.character_positions[last] + run.character_widths[last];
    assert!(
        (right - run.bounds.x1).abs() < 0.01,
        "the run ends at its last character"
    );
}

#[test]
fn the_runs_move_with_the_origin() {
    TextSystem::bundled_only().install();
    let layout = TextLayout::new("abc", &TextStyle::default(), None);
    let at_zero = text_runs(&layout.layout, "abc", Point::default(), None);
    let moved = text_runs(&layout.layout, "abc", Point { x: 10.0, y: 20.0 }, None);
    assert_eq!(moved[0].bounds.x0, at_zero[0].bounds.x0 + 10.0);
    assert_eq!(moved[0].bounds.y0, at_zero[0].bounds.y0 + 20.0);
    assert_eq!(moved[0].character_positions, at_zero[0].character_positions);
}

#[test]
fn a_right_to_left_stretch_is_a_run_of_its_own_on_the_same_line() {
    let text = "abc \u{5d0}\u{5d1}\u{5d2} def";
    let runs = runs_of(text, None);
    let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(joined, text, "the runs read in logical order");
    let hebrew = runs
        .iter()
        .position(|run| run.right_to_left)
        .expect("the hebrew stretch reads right to left");
    assert!(runs[hebrew].text.contains('\u{5d0}'));
    assert!(!runs[0].right_to_left);
    assert!(!runs[0].continues_line, "the first run starts the line");
    assert!(
        runs[1..].iter().all(|run| run.continues_line),
        "the other stretches go on with it"
    );
    let positions = &runs[hebrew].character_positions;
    for pair in positions.windows(2) {
        assert!(
            pair[1] > pair[0],
            "each letter sits further from the right edge: {positions:?}"
        );
    }
}

#[test]
fn a_run_knows_where_its_text_starts() {
    let text = format!("ab\n{}", "x".repeat(TEXT_RUN_CAP + 3));
    let runs = runs_of(&text, None);
    let mut at = 0;
    for run in &runs {
        assert_eq!(run.start, at);
        assert_eq!(&text[run.start..run.start + run.text.len()], run.text);
        at += run.text.len();
    }
}

#[test]
fn the_stretches_of_a_long_line_go_on_with_it_and_the_next_line_does_not() {
    let text = format!("{}\nnext", "x".repeat(TEXT_RUN_CAP + 3));
    let runs = runs_of(&text, None);
    let flags: Vec<bool> = runs.iter().map(|run| run.continues_line).collect();
    assert_eq!(flags, vec![false, true, false]);
}

#[test]
fn what_a_truncation_hides_sits_at_the_cut_with_no_width() {
    TextSystem::bundled_only().install();
    let text = "several words that wrap onto more than one line";
    let layout = TextLayout::new(text, &TextStyle::default(), Some(120.0));
    let shown = text_runs(&layout.layout, text, Point::default(), None);
    let first_line = shown[0].bounds;
    let cut = first_line.x0 + 30.0;
    let runs = text_runs(
        &layout.layout,
        text,
        Point::default(),
        Some(Hidden {
            kept_lines: 1,
            cut: Some(cut),
        }),
    );
    let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(joined, text, "the whole text is still read");
    for run in &runs {
        assert_eq!(
            (run.bounds.y0, run.bounds.y1),
            (first_line.y0, first_line.y1),
            "every run sits on the kept line"
        );
        assert!(run.bounds.x1 <= cut + 0.01, "no run reaches past the cut");
    }
    assert!(runs[1..].iter().all(|run| run.continues_line));
    let hidden = runs.last().expect("the hidden lines give runs");
    assert!(hidden.character_widths.iter().all(|width| *width == 0.0));
}

#[test]
fn the_stretches_of_a_mixed_line_read_in_logical_order() {
    let text = "abc \u{5d0}\u{5d1}\u{5d2} 123 \u{5d3}\u{5d4}\u{5d5} def";
    let runs = runs_of(text, None);
    assert!(runs.len() > 3, "the line divides by direction: {runs:?}");
    let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(joined, text);
}

/// The runs of `text` laid out at `width` with the lines past `kept_lines`
/// hidden and no cut, as a truncation that drops lines leaves them, and the
/// runs of the same layout shown whole.
fn hidden_runs(text: &str, width: Option<f32>, kept_lines: usize) -> (Vec<TextRun>, Vec<TextRun>) {
    TextSystem::bundled_only().install();
    let layout = TextLayout::new(text, &TextStyle::default(), width);
    let shown = text_runs(&layout.layout, text, Point::default(), None);
    let hidden = text_runs(
        &layout.layout,
        text,
        Point::default(),
        Some(Hidden {
            kept_lines,
            cut: None,
        }),
    );
    (shown, hidden)
}

#[test]
fn a_hidden_line_after_a_hard_break_starts_a_line_of_its_own() {
    let (_, runs) = hidden_runs("ab\ncd ef", None, 1);
    let flags: Vec<(&str, bool)> = runs
        .iter()
        .map(|run| (run.text.as_str(), run.continues_line))
        .collect();
    assert_eq!(flags, vec![("ab\n", false), ("cd ef", false)]);
    assert!(runs[1].character_widths.iter().all(|width| *width == 0.0));
}

#[test]
fn a_label_that_shows_no_line_folds_all_of_it_to_where_it_starts() {
    let (shown, runs) = hidden_runs("several words that wrap", Some(80.0), 0);
    assert!(
        runs.iter()
            .all(|run| run.character_widths.iter().all(|width| *width == 0.0))
    );
    assert!(
        runs.iter()
            .all(|run| run.bounds.x0 == run.bounds.x1 && run.bounds.x0 == shown[0].bounds.x0)
    );
}

#[test]
fn hidden_right_to_left_text_sits_where_its_last_kept_line_ends() {
    let text =
        "\u{5d0}\u{5d1}\u{5d2} \u{5d3}\u{5d4}\u{5d5} \u{5d6}\u{5d7}\u{5d8} \u{5d9}\u{5da}\u{5db}";
    let (shown, runs) = hidden_runs(text, Some(40.0), 1);
    let first_line_left = shown
        .iter()
        .filter(|run| run.bounds.y0 == shown[0].bounds.y0)
        .map(|run| run.bounds.x0)
        .fold(f32::INFINITY, f32::min);
    let folded: Vec<&TextRun> = runs
        .iter()
        .filter(|run| run.character_widths.iter().all(|width| *width == 0.0))
        .collect();
    assert!(!folded.is_empty(), "the text wraps past the first line");
    assert!(
        folded
            .iter()
            .all(|run| run.bounds.x1 <= first_line_left + 0.01),
        "the hidden text waits at the left, where a right to left line ends"
    );
}
