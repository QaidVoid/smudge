//! The live theme file: a plain text file of `token = value` lines that a
//! running app reads back whenever it changes.
//!
//! One line sets one token of the theme: a color as `#rrggbb` or
//! `#rrggbbaa`, or a corner radius as a plain number of logical pixels. The
//! token names are the keys of [`Theme::colors`] and [`Theme::radii`]. Blank
//! lines and everything after a `#` that starts a line are ignored. A line
//! that names no token of the theme, or whose value does not parse, is
//! skipped and reported.
//!
//! ```text
//! # A warmer accent.
//! accent = #e07a3a
//! radius_md = 10
//! ```

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use smudge_core::geometry::Color;
use smudge_core::reactive::{RwSignal, prelude::*};
use smudge_core::theme::Theme;
use smudge_core::timer::{self, TimerHandle};

/// How often [`watch_theme`] looks at the file.
const POLL: Duration = Duration::from_millis(250);

thread_local! {
    /// The file [`watch_theme`] watches and the theme it writes, which the
    /// panel's "Write theme" button reads.
    static WATCHED: RefCell<Option<(PathBuf, RwSignal<Theme>)>> = const { RefCell::new(None) };
    /// Where the lines a read of the watched file skipped go: the signal of
    /// the devtools panel of this thread, which shows them.
    static REPORT: RefCell<Option<RwSignal<Vec<String>>>> = const { RefCell::new(None) };
}

/// Watches the theme file at `path`, and writes `theme` with the file's
/// tokens over the theme it holds now, whenever the file changes.
///
/// The app binds the same signal with
/// [`App::bind_theme`](smudge_core::app::App::bind_theme), so an edit
/// recolors it within a quarter of a second. The file is polled on the app's
/// own thread, from a timer of the owner that calls this, so call it from
/// the app's build or its configuration. Each read starts from the theme the
/// signal held when watching began, so a line deleted from the file puts its
/// token back. A file that cannot be read leaves the theme as it is. The
/// lines a read skipped show in the devtools panel.
///
/// Watching stops when the returned handle is dropped.
///
/// ```no_run
/// use smudge_core::reactive::RwSignal;
/// use smudge_core::theme::Theme;
///
/// let theme = RwSignal::new(Theme::dark());
/// let _watch = smudge_devtools::watch_theme("theme.txt", theme.clone());
/// ```
pub fn watch_theme(path: impl Into<PathBuf>, theme: RwSignal<Theme>) -> TimerHandle {
    let path = path.into();
    let base = theme.get();
    WATCHED.with(|watched| *watched.borrow_mut() = Some((path.clone(), theme.clone())));
    let mut seen: Option<SystemTime> = None;
    timer::every(POLL, move || {
        let modified = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok();
        if modified.is_none() || modified == seen {
            return;
        }
        seen = modified;
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let mut next = base.clone();
        let skipped = apply_theme_file(&text, &mut next);
        theme.set(next);
        if let Some(report) = REPORT.with(|report| report.borrow().clone()) {
            report.set(skipped);
        }
    })
}

/// Sets the tokens of `theme` that `text`, in the theme file format, names,
/// and returns a line for each line it skipped, saying why.
pub fn apply_theme_file(text: &str, theme: &mut Theme) -> Vec<String> {
    let mut skipped = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let Some((token, value)) = line.split_once('=') else {
            skipped.push(format!("line {number}: `{line}` is not `token = value`"));
            continue;
        };
        let (token, value) = (token.trim(), value.trim());
        if let Some(color) = theme.colors.get_mut(token) {
            match parse_color(value) {
                Some(parsed) => *color = parsed,
                None => skipped.push(format!(
                    "line {number}: `{value}` is not a color like #rrggbb or #rrggbbaa"
                )),
            }
        } else if let Some(radius) = theme.radii.get_mut(token) {
            match value.parse::<f32>() {
                Ok(parsed) if parsed.is_finite() => *radius = parsed,
                _ => skipped.push(format!("line {number}: `{value}` is not a number")),
            }
        } else {
            skipped.push(format!("line {number}: the theme has no token `{token}`"));
        }
    }
    skipped
}

/// Writes `theme` in the theme file format: its colors and then its radii,
/// each sorted by token.
pub fn theme_file(theme: &Theme) -> String {
    let mut text = String::from("# Colors, as #rrggbb or #rrggbbaa.\n");
    let mut colors: Vec<_> = theme.colors.iter().collect();
    colors.sort_by_key(|(token, _)| **token);
    for (token, color) in colors {
        let _ = write!(
            text,
            "{token} = #{:02x}{:02x}{:02x}",
            color.r, color.g, color.b
        );
        if color.a != 255 {
            let _ = write!(text, "{:02x}", color.a);
        }
        text.push('\n');
    }
    text.push_str("# Corner radii, in logical pixels.\n");
    let mut radii: Vec<_> = theme.radii.iter().collect();
    radii.sort_by_key(|(token, _)| **token);
    for (token, radius) in radii {
        let _ = writeln!(text, "{token} = {radius}");
    }
    text
}

/// Returns the theme file [`watch_theme`] watches and the theme it writes,
/// when one is watched on this thread.
pub(crate) fn watched() -> Option<(PathBuf, RwSignal<Theme>)> {
    WATCHED.with(|watched| watched.borrow().clone())
}

/// Makes `report` the signal the lines a read skipped go to.
pub(crate) fn report_to(report: RwSignal<Vec<String>>) {
    REPORT.with(|slot| *slot.borrow_mut() = Some(report));
}

/// Parses `#rrggbb` or `#rrggbbaa`.
fn parse_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    let alpha = if hex.len() == 8 { channel(6)? } else { 255 };
    Some(Color::rgba(channel(0)?, channel(2)?, channel(4)?, alpha))
}

#[cfg(test)]
#[path = "theme_file_tests.rs"]
mod tests;
