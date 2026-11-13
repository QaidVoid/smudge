# Known limits

<p class="lead">What Smudge does not do yet, so you can decide before you build on it.</p>

## Widgets

- **Rich text spans.** `label` styles one run of text and `text_area` edits plain multi-line text. Text with mixed bold, monospace or link runs inside one paragraph is not there yet.
- **Vertical scrolling only.** The scroll view scrolls up and down. Content wider than the view is clipped, not scrolled.
- **The scroll thumb is an indicator.** It shows the position but cannot be dragged.
- **No popups that escape a clip.** There is no overlay layer yet, so a menu, tooltip or dropdown is clipped by any ancestor that clips.
- **Undo lives in each input.** Every text input keeps its own undo history, cleared when the application replaces the text. There is no application-wide undo.

## Drawing

- **Rectangular clips.** A panel with rounded corners that clips its children clips them to a rectangle.
- **Clip edges are whole pixels.** At a fractional clip edge, a child may draw into a pixel the edge only partly covers.
- **No color emoji on the CPU renderer.** A glyph from a color font draws as its plain outline, which for most emoji is nothing. The GPU renderer draws color glyphs.
- **No variable font instances, no synthetic bold or italic.** A variable font draws at its default instance, and a weight the font has no face for draws without bolding.
- **Large images are softer on the GPU.** The GPU renderer packs images into one atlas of at most 8192 by 8192 pixels, and the device's largest texture can lower that. An image with a longer side is downscaled once to fit, keeping its aspect ratio, so the GPU renderer draws it at a lower resolution than the CPU renderer does. The atlas holds all of a frame's images together, so several images near the limit in one frame can still leave one out.
- **The GPU renderer redraws the whole frame.** Damage tracking still decides whether a frame is needed at all, but a GPU frame draws everything.

## Text

- **Scripts without spaces need a feature.** Thai, Lao, Khmer and Burmese wrap only at spaces unless `complex-scripts` is on. See [Text and input](/guide/text-and-input#scripts-without-spaces).
- **Right-to-left labels truncate without an ellipsis.** A label capped with `max_lines` cuts its last line at the visually trailing edge and ends it in an ellipsis. A layout whose dominant direction is right-to-left keeps the cap but shows its kept lines whole, with no ellipsis, since the ellipsis would have to lead the line rather than close it.
- **The ellipsis needs a font that maps it.** Truncation uses the horizontal ellipsis of the cut run's font, or three full stops when that font lacks the ellipsis but maps a full stop. When the run's font maps neither, it uses the ellipsis of the label's own font. When that font maps neither either, the label still keeps its cap and cuts its last line, but ends it without an ellipsis.

## Platforms

- **Tested on Linux.** The window layer supports Windows and macOS, but Smudge is built and tested on Linux under Wayland.
- **No clipboard on the web.** Copy and paste work only inside the application there.
- **No screen readers on the web.** Browsers expose their own accessibility tree, and projecting into it needs a different approach.
- **Wayland clipboard needs data-control or Xwayland.** On a compositor with neither, copy and paste stay inside the application.

## Stability

Smudge follows semantic versioning, and before 1.0 a minor version may change the API. Breaking changes are listed in the changelog of each release.
