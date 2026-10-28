# Known limits

<p class="lead">What v0.1 does not do yet, so you can decide before you build on it.</p>

## Widgets

- **One line of text input.** `text_input` edits a single line. There is no multi-line editor or rich text yet.
- **Vertical scrolling only.** The scroll view scrolls up and down. Content wider than the view is clipped, not scrolled.
- **The scroll thumb is an indicator.** It shows the position but cannot be dragged.
- **No popups that escape a clip.** There is no overlay layer yet, so a menu, tooltip or dropdown is clipped by any ancestor that clips.
- **Undo lives in each input.** Every text input keeps its own undo history, cleared when the application replaces the text. There is no application-wide undo.

## Drawing

- **Rectangular clips.** A panel with rounded corners that clips its children clips them to a rectangle.
- **Clip edges are whole pixels.** At a fractional clip edge, a child may draw into a pixel the edge only partly covers.
- **No color emoji on the CPU renderer.** A glyph from a color font draws as its plain outline, which for most emoji is nothing. The GPU renderer draws color glyphs.
- **No variable font instances, no synthetic bold or italic.** A variable font draws at its default instance, and a weight the font has no face for draws without bolding.
- **The GPU renderer redraws the whole frame.** Damage tracking still decides whether a frame is needed at all, but a GPU frame draws everything.

## Text

- **Scripts without spaces need a feature.** Thai, Lao, Khmer and Burmese wrap only at spaces unless `complex-scripts` is on. See [Text and input](/guide/text-and-input#scripts-without-spaces).
- **Font types come from parley.** Setting a label's weight or family names `FontWeight` and `FontFamily` from the `parley` crate.

## Platforms

- **Tested on Linux.** The window layer supports Windows and macOS, but v0.1 was built and tested on Linux under Wayland.
- **No clipboard on the web.** Copy and paste work only inside the application there.
- **No screen readers on the web.** Browsers expose their own accessibility tree, and projecting into it needs a different approach.
- **Wayland clipboard needs data-control or Xwayland.** On a compositor with neither, copy and paste stay inside the application.

## Stability

Smudge follows semantic versioning, and before 1.0 a minor version may change the API. Breaking changes are listed in the changelog of each release.
