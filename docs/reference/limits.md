# Known limits

<p class="lead">What Smudge does not do yet, so you can decide before you build on it.</p>

## Widgets

- **Selections start from the scope's own presses.** A row inside a selection scope that takes presses itself, with a press or hover handler or a hover style, keeps them, so a selection cannot start on it. A press on a link inside a scope takes the keyboard focus, so copying needs a press on the scope's text again.
- **Rich text is read-only and per paragraph.** `rich_text` shows mixed bold, italic, monospace, colored and link runs in one paragraph, and a selection scope selects and copies it with the pointer and the keyboard. A selection end the keyboard moves into a block a virtual list has not mounted, as Ctrl+End does, is not scrolled into view. Its links are not reachable with the keyboard, `text_area` edits plain text only, and spans share one font size.
- **Markdown leaves some of itself as text.** An HTML block shows its source, an image its description, and a long line of code wraps, since a scroll view does not scroll sideways. Lists and quotes nested more than 32 deep show what they hold at the 32nd level. Math, footnotes and task lists are not recognized, so they read as the CommonMark they are: mostly plain text, though a footnote whose body is a single destination is a link definition. The parser limits how much text references may expand to, to the length of the text it reads or 100 KB, whichever is larger. A streamed document counts that limit for each block on its own, so where references expand to more than one of the two limits allows, a block can show links where the parse of the whole shows text, or text where it shows links.
- **Vertical scrolling only.** The scroll view scrolls up and down. Content wider than the view is clipped, not scrolled.
- **The scroll thumb is an indicator.** It shows the position but cannot be dragged.
- **No popups that escape a clip.** There is no overlay layer yet, so a menu, tooltip or dropdown is clipped by any ancestor that clips.
- **Undo lives in each input.** Every text input keeps its own undo history, cleared when the application replaces the text. There is no application-wide undo.

## Drawing

- **Rectangular clips.** A panel with rounded corners that clips its children clips them to a rectangle.
- **Clip edges are whole pixels.** At a fractional clip edge, a child may draw into a pixel the edge only partly covers.
- **No color emoji on the CPU renderer.** A glyph from a color font draws as its plain outline, which for most emoji is nothing. The GPU renderer draws color glyphs.
- **Static faces imitate the styles they lack, and a slant axis is not used.** A variable font draws at the weight the text asks for through its weight axis, and draws its italic through an italic axis. One whose slant comes from a slant axis alone draws its italic upright. A family of static faces draws an italic that none of its faces has by slanting the closest face, and a weight more than 200 above the closest face's by widening it, which comes close to a bold but leaves a medium or a semibold at the regular weight.
- **The GPU renderer draws images from a size-matched copy.** It packs the images of a frame into one atlas of at most 8192 by 8192 pixels, and the device's largest texture can lower that. So each image is handed over at the size it is drawn at, rounded up to a power of two and capped at that limit, and images outside the window or their clips are left out. An image larger than that size is downscaled to fit, keeping its aspect ratio, so it can look slightly different from the CPU renderer's drawing of the full image. The copy is made again when the image comes back after a few frames out of view. An image drawn bigger than 8192 pixels on a side is softer on the GPU, and visible images whose drawn sizes add up to more than the atlas holds can still leave one out.
- **The GPU renderer redraws the whole frame.** Damage tracking still decides whether a frame is needed at all, but a GPU frame draws everything.

## Text

- **Scripts without spaces need a feature.** Thai, Lao, Khmer and Burmese wrap only at spaces unless `complex-scripts` is on. See [Text and input](/guide/text-and-input#scripts-without-spaces).
- **Right-to-left labels truncate without an ellipsis.** A label capped with `max_lines` cuts its last line at the visually trailing edge and ends it in an ellipsis. A layout whose dominant direction is right-to-left keeps the cap but shows its kept lines whole, with no ellipsis, since the ellipsis would have to lead the line rather than close it.
- **Fonts of the application's own are registered by hand.** A build whose only font source is `app-fonts` draws no text until the application registers a font, and nothing reports a forgotten registration. Register fonts before the window opens. See [Shipping a small binary](/guide/small-binary#app-fonts).
- **The ellipsis needs a font that maps it.** Truncation uses the horizontal ellipsis of the cut run's font, or three full stops when that font lacks the ellipsis but maps a full stop. When the run's font maps neither, it uses the ellipsis of the label's own font. When that font maps neither either, the label still keeps its cap and cuts its last line, but ends it without an ellipsis.

## Platforms

- **Tested on Linux.** The window layer supports Windows and macOS, but Smudge is built and tested on Linux under Wayland.
- **No clipboard on the web.** Copy and paste work only inside the application there.
- **No screen readers on the web.** Browsers expose their own accessibility tree, and projecting into it needs a different approach.
- **Maximize under X11 depends on the window manager.** `toggle_maximize` and `maximized` use the standard X11 maximize request and state. A window manager that does not implement them, as some X11 bridges inside Wayland compositors do not, ignores the request, and `maximized` stays `false`. Running natively on Wayland avoids this.
- **Wayland clipboard needs data-control or Xwayland.** On a compositor with neither, copy and paste stay inside the application.
- **The color scheme follows the desktop on Linux, macOS and Windows.** An app that follows the system's light or dark preference reads the desktop's `color-scheme` setting through the settings portal on the session bus on Linux, and the theme the window system reports on macOS and Windows. A desktop that serves no portal, and the web, report nothing: the app keeps the light side it starts on until a report says otherwise.
- **Reduced motion follows Windows and macOS at focus.** The window reads the reduced-motion setting from the settings portal on Linux, which xdg-desktop-portal serves from version 1.21, and from the `prefers-reduced-motion` media query on the web, and follows their changes at once. Windows and macOS announce no change to the window, so there it reads the setting when it starts and again whenever it regains the focus, and a change made while the app stays in front takes effect at the next focus.

## Hot reload

- **x86-64 Linux only.** `cargo smudge dev` patches a running app on x86-64 Linux. Elsewhere `cargo smudge new` and `cargo smudge init` work, and `dev` refuses.
- **The build directory must be the target directory.** A cargo configuration that moves the build directory away from the target directory, through `build.build-dir` or `CARGO_BUILD_BUILD_DIR`, keeps patches from finding the workspace's libraries.
- **Statics of the patched app crate start fresh.** The patched code reaches a static of the app crate, `pub` or not, by a short distance from itself, and the patch is loaded too far from the running app to reach the app's copy, so a writable one starts over at its initial value in the patched code. `cargo smudge dev` names such statics once per session. Keep state that must survive a patch in the state passed to `run_view`, in signals, or in a `pub static` of a library crate: a patch shares those with the running app, as long as the static's type keeps its layout. An edited immutable static, such as a table or a string, shows its new value.
- **Some library statics ask for a restart.** An edit of a library crate whose writable statics a patch cannot share, because they are private, their type changed, or the debug information does not describe them, asks for a restart.
- **Closures that trade places can run each other's bodies.** A closure the app keeps outside the tree, such as one a configure step, a timer or a worker holds, runs the patched body of the closure that now has its place when two closures of a function swap, or when one is removed and another added so their count stays the same. When the count of a function's closures and async blocks changes, the change is detected, and those closures keep their old bodies until a restart.

## Stability

Smudge follows semantic versioning, and before 1.0 a minor version may change the API. Breaking changes are listed in the changelog of each release.
