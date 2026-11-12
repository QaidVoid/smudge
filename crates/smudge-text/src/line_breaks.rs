//! The paste rule of the single-line input: line breaks become spaces.
//!
//! The module depends on nothing but the standard library, so the fuzz
//! target includes this same file and checks the rule the input applies.

/// Returns whether `character` forces a line break: a line feed, a carriage
/// return, a vertical tab, a form feed, a next line, or the line and
/// paragraph separators.
pub(crate) fn is_line_break(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Replaces every line break in `text` with one space.
///
/// A carriage return immediately followed by a line feed counts as one
/// break, so a Windows paste gains no extra space where its text had one
/// line break.
pub(crate) fn flatten_line_breaks(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if !is_line_break(character) {
            line.push(character);
            continue;
        }
        line.push(' ');
        if character == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
        }
    }
    line
}
