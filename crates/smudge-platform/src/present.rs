//! Drawing or copying a frame into the window buffer, and choosing how much
//! of the surface a present has to carry.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::app::FrameOutput;
use smudge_core::geometry::PixelRect;
use smudge_cpu::{CpuRenderer, XrgbRenderer};
use web_time::Instant;

/// How many past frames the damage ring keeps.
///
/// Softbuffer reports ages of at most 3 for the double and triple buffered
/// backends this library targets, so a longer ring could never be read.
const HISTORY: usize = 3;

/// How many presents in a row may find a buffer of unknown age before the
/// platform counts as one that never reports buffer ages.
///
/// A surface reports age 0 for each of its buffers after it was created or
/// resized. A triple-buffered one does so for three presents in a row, so
/// only a fourth present in a row at age 0 shows that the ages are unknown.
const UNKNOWN_AGE_PRESENTS: u8 = 4;

/// Where the CPU renderer draws a frame before the window shows it.
enum CpuTarget {
    /// Straight into the window buffer, which then holds the only copy of
    /// the frame.
    ///
    /// The buffer's age says which frame it holds, and the rectangles that
    /// changed since are drawn again from the current frame. This is the
    /// default wherever the platform reports buffer ages, and it saves the
    /// memory of a second frame.
    Direct(XrgbRenderer),
    /// Into a pixmap that survives between frames, copied into the window
    /// buffer on every present.
    ///
    /// A platform that never reports a buffer's age gives no way to know what
    /// a buffer misses, so every present there carries the whole frame, and
    /// copying it from the pixmap is much cheaper than drawing it again. The
    /// damage overlay uses this target too, so its tint never reaches a
    /// pixel a later frame keeps.
    Pixmap(CpuRenderer),
}

/// The CPU renderer's part of a window's presents: it draws each frame into
/// the window buffer it is handed, from that buffer's age and the damage of
/// the frames presented before.
///
/// It holds no window or surface. The caller acquires a buffer, passes it to
/// [`CpuFrames::draw`] with its age, presents it, and then reports the
/// present with [`CpuFrames::presented`].
pub(crate) struct CpuFrames {
    /// How the frames reach the window buffer.
    target: CpuTarget,
    /// The surface width in physical pixels.
    width: u32,
    /// The surface height in physical pixels.
    height: u32,
    /// Whether every present tints the frame's damage in the window buffer.
    show_damage: bool,
    /// The damage of the frames the window presented, most recent first.
    history: DamageHistory,
    /// The region the last drawn frame presents with damage.
    region: Vec<PixelRect>,
    /// How many presents in a row, since the surface took its size, found a
    /// window buffer of unknown age.
    unknown_ages: u8,
    /// Whether the pixmap holds nothing it can keep, because it was just
    /// created or resized, so the next frame repaints all of it.
    pixmap_stale: bool,
}

/// What drawing one frame into a window buffer did.
pub(crate) struct Drawn {
    /// Whether the present carries only [`CpuFrames::region`], rather than
    /// the whole surface.
    pub(crate) with_damage: bool,
    /// How long the renderer took to draw the frame.
    pub(crate) raster: Duration,
    /// How long copying the frame from the pixmap took, zero when it was
    /// drawn straight into the buffer.
    pub(crate) copy: Duration,
    /// How many bytes were copied from the pixmap.
    pub(crate) bytes_copied: u64,
}

impl CpuFrames {
    /// Creates the renderer for a surface of `width` by `height` physical
    /// pixels.
    ///
    /// Frames are drawn straight into the window buffer until
    /// [`UNKNOWN_AGE_PRESENTS`] presents in a row find a buffer of unknown
    /// age, and from then on through a pixmap, as [`CpuTarget`] explains.
    /// With `show_damage`, they always go through the pixmap.
    pub(crate) fn new(width: u32, height: u32, show_damage: bool) -> CpuFrames {
        let target = if show_damage {
            CpuTarget::Pixmap(CpuRenderer::new(width, height))
        } else {
            CpuTarget::Direct(XrgbRenderer::new(width, height))
        };
        CpuFrames {
            target,
            width,
            height,
            show_damage,
            history: DamageHistory::default(),
            region: Vec::new(),
            unknown_ages: 0,
            pixmap_stale: true,
        }
    }

    /// Takes the surface's new size.
    ///
    /// The frames presented before are forgotten, because a resized surface's
    /// buffers hold none of them, so the next presents draw everything until
    /// new frames are recorded.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        match &mut self.target {
            CpuTarget::Direct(renderer) => renderer.resize(width, height),
            CpuTarget::Pixmap(renderer) => renderer.resize(width, height),
        }
        self.width = width;
        self.height = height;
        self.unknown_ages = 0;
        self.pixmap_stale = true;
        self.history.clear();
    }

    /// Whether `frame` must skip its present. See [`present_skipped`].
    pub(crate) fn skips(&self, frame: &FrameOutput<'_>, has_tick: bool) -> bool {
        present_skipped(frame.damage, has_tick, self.show_damage)
    }

    /// Returns the region the last drawn frame presents when
    /// [`Drawn::with_damage`] is set.
    pub(crate) fn region(&self) -> &[PixelRect] {
        &self.region
    }

    /// Draws `frame` into `buffer`, a window buffer whose contents are the
    /// frame presented `age` presents ago, or unknown at age 0.
    ///
    /// The frame's damage and the recorded damage of the frames the buffer
    /// missed decide what it is missing: when the buffer's age says which
    /// frame it holds and no full repaint happened since, only their union
    /// is drawn into the buffer, or copied from the pixmap, and the present
    /// carries it as damage. Every other case draws or copies the whole
    /// surface and presents all of it. A frame with no damage is the
    /// exception on a buffer of known age: it draws only what the buffer
    /// misses, because the buffer kept its pixels, and presents everything
    /// to restore a window that lost its contents. While the damage overlay
    /// is on, every present copies the whole pixmap and blends the tint over
    /// the frame's damage in the window buffer, so no tint reaches the pixmap
    /// a later frame repaints from.
    ///
    /// # Panics
    ///
    /// Panics when `buffer` does not hold one entry per pixel of the surface,
    /// or when the frame's damage reaches outside it, both of which mean the
    /// frame was produced for a surface of another size.
    pub(crate) fn draw(&mut self, frame: &FrameOutput<'_>, age: u8, buffer: &mut [u32]) -> Drawn {
        let whole = PixelRect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        };
        self.unknown_ages = if age == 0 {
            self.unknown_ages.saturating_add(1)
        } else {
            0
        };
        if matches!(self.target, CpuTarget::Direct(_)) && self.unknown_ages >= UNKNOWN_AGE_PRESENTS
        {
            self.target = CpuTarget::Pixmap(CpuRenderer::new(self.width, self.height));
            self.pixmap_stale = true;
        }
        let raster_start = Instant::now();
        match &mut self.target {
            CpuTarget::Direct(renderer) => {
                let with_damage = draw_direct(
                    renderer,
                    frame,
                    age,
                    &self.history,
                    &mut self.region,
                    buffer,
                );
                Drawn {
                    with_damage,
                    raster: raster_start.elapsed(),
                    copy: Duration::ZERO,
                    bytes_copied: 0,
                }
            }
            CpuTarget::Pixmap(renderer) => {
                if self.pixmap_stale {
                    renderer.render(&FrameOutput {
                        damage: &[whole],
                        ..*frame
                    });
                    self.pixmap_stale = false;
                } else {
                    renderer.render(frame);
                }
                let raster = raster_start.elapsed();
                let copy_start = Instant::now();
                debug_assert_eq!(
                    buffer.len(),
                    renderer.pixels().len() / 4,
                    "the window buffer must hold one entry per rendered pixel"
                );
                let with_damage = !self.show_damage
                    && damage_region(
                        age,
                        frame.damage,
                        frame.full,
                        &self.history,
                        &mut self.region,
                    );
                if !with_damage {
                    self.region.clear();
                    self.region.push(whole);
                }
                let mut bytes_copied = 0;
                for &rect in &self.region {
                    bytes_copied += u64::from(rect.width) * u64::from(rect.height) * 4;
                    copy_rect(renderer.pixels(), self.width, buffer, rect);
                }
                if self.show_damage {
                    tint_rects(buffer, self.width, frame.damage);
                }
                Drawn {
                    with_damage,
                    raster,
                    copy: copy_start.elapsed(),
                    bytes_copied,
                }
            }
        }
    }

    /// Records that the buffer [`CpuFrames::draw`] drew `frame` into was
    /// presented.
    ///
    /// A frame that was drawn but never presented must not be recorded,
    /// because no buffer holds it.
    pub(crate) fn presented(&mut self, frame: &FrameOutput<'_>) {
        self.history.push(frame.damage, frame.full);
    }
}

/// What the last present did.
///
/// The platform records one of these after every frame it presents and, when
/// the window was opened with a [`PresentStatsHandle`], publishes it there for
/// the application to read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentStats {
    /// The buffer age the platform reported, in presents.
    ///
    /// An age of 0 means the buffer contents were unknown and the present
    /// copied the whole surface.
    pub buffer_age: u8,
    /// Whether the present carried only the damaged region.
    ///
    /// A present without damage copied and presented the whole surface.
    pub with_damage: bool,
    /// How many surface bytes the present copied from the renderer's pixmap.
    ///
    /// Four bytes per pixel, so a full present of a 100 by 100 surface copies
    /// 40,000 bytes. The CPU path keeps a pixmap only where the platform
    /// reports no buffer ages, and while the damage overlay is on. Everywhere
    /// else it draws straight into the window buffer and, like the GPU path,
    /// reports none.
    pub bytes_copied: u64,
    /// The width of the presented surface in physical pixels.
    pub surface_width: u32,
    /// The height of the presented surface in physical pixels.
    pub surface_height: u32,
    /// How long the renderer took to paint this frame's damage into its
    /// surface.
    ///
    /// When the CPU path draws straight into the window buffer, this covers
    /// everything the buffer was missing, the rectangles an older buffer
    /// missed included, and the conversion to the window's pixel format. On
    /// the GPU path this is the whole draw, whose submission reaches the GPU
    /// asynchronously.
    pub raster: Duration,
    /// How long the platform waited for a free window buffer before the
    /// present could start.
    ///
    /// A compositor that still holds the previous buffers makes this wait,
    /// often for up to one display refresh. It is time spent blocked, not
    /// work.
    pub acquire: Duration,
    /// How long the present took, from holding a free window buffer to handing
    /// it back to the platform: the copy, if any, and the submission, without
    /// the wait that [`PresentStats::acquire`] holds or the drawing that
    /// [`PresentStats::raster`] holds.
    pub present: Duration,
    /// Which renderer presented the frame, as its name: `cpu` or `gpu`.
    ///
    /// Empty until the first present.
    pub renderer: &'static str,
}

/// A shared slot the platform writes the latest [`PresentStats`] into.
///
/// Create one, clone it into the `stats` field of the window options, and keep
/// the original to read the statistics of the most recent present while the
/// window runs. The handle reports zeros until the first present.
///
/// # Examples
///
/// ```
/// use smudge_platform::PresentStatsHandle;
///
/// let handle = PresentStatsHandle::new();
/// let reader = handle.clone();
/// assert_eq!(reader.get().buffer_age, 0);
/// ```
#[derive(Clone, Debug, Default)]
pub struct PresentStatsHandle {
    /// The statistics of the most recent present.
    slot: Rc<RefCell<PresentStats>>,
    /// The GPU adapter the window presents through, reported once when the
    /// GPU path is created.
    #[cfg(feature = "gpu")]
    adapter: Rc<RefCell<Option<crate::AdapterSummary>>>,
}

impl PresentStatsHandle {
    /// Creates a handle that reports zeros until the first present.
    pub fn new() -> PresentStatsHandle {
        PresentStatsHandle::default()
    }

    /// Returns the statistics of the most recent present.
    pub fn get(&self) -> PresentStats {
        *self.slot.borrow()
    }

    /// Publishes the statistics of the most recent present.
    pub(crate) fn store(&self, stats: PresentStats) {
        *self.slot.borrow_mut() = stats;
    }

    /// Returns the GPU adapter the window presents through, when it knows
    /// one.
    ///
    /// The platform reports the adapter once, when it sets the GPU
    /// presentation path up, and only when the window was opened with a
    /// handle in its options. A window on the CPU renderer reports `None`,
    /// which is how a reader tells a GPU window from a CPU one.
    #[cfg(feature = "gpu")]
    pub fn adapter(&self) -> Option<crate::AdapterSummary> {
        self.adapter.borrow().clone()
    }

    /// Reports the adapter of the GPU presentation path.
    #[cfg(feature = "gpu")]
    pub(crate) fn set_adapter(&self, adapter: crate::AdapterSummary) {
        *self.adapter.borrow_mut() = Some(adapter);
    }
}

/// Copies one rectangle of the surface into the window buffer.
///
/// `source` is the renderer's pixmap as premultiplied RGBA8, row-major with no
/// padding between rows, `source_width` pixels per row. `destination` is the
/// window buffer, one `u32` per pixel, with the same width as the pixmap. Each
/// destination entry becomes `0x00RRGGBB`: the alpha channel is dropped, so a
/// non-opaque clear color is unsupported and the copy would lose it. Pixels
/// outside `rect` are left alone.
///
/// # Panics
///
/// Panics when `rect` reaches outside the surface, which means the frame was
/// produced for a surface of a different size. That is a broken invariant of
/// the caller, not input from outside the library.
pub(crate) fn copy_rect(
    source: &[u8],
    source_width: u32,
    destination: &mut [u32],
    rect: PixelRect,
) {
    let stride = source_width as usize;
    let width = rect.width as usize;
    for y in rect.y..rect.y + rect.height {
        let source_row = &source[(y as usize * stride + rect.x as usize) * 4..][..width * 4];
        let destination_row = &mut destination[y as usize * stride + rect.x as usize..][..width];
        for (pixel, rgba) in destination_row
            .iter_mut()
            .zip(source_row.as_chunks::<4>().0)
        {
            *pixel = u32::from(rgba[0]) << 16 | u32::from(rgba[1]) << 8 | u32::from(rgba[2]);
        }
    }
}

/// Draws everything a window buffer of `age` is missing for `frame` straight
/// into `buffer`, and returns whether only `region` needs presenting.
///
/// The region is the one [`missing_region`] chooses, and the whole surface
/// when that region is unknown. The buffer then ends up equal to a full
/// repaint of the frame, because a rectangle the buffer missed is drawn
/// again from the frame being presented rather than copied from a kept
/// surface. The result follows [`damage_region`]: a frame with no damage
/// presents the whole buffer to restore a window that lost its contents, but
/// draws only what the buffer misses, because the buffer kept its pixels.
///
/// # Panics
///
/// Panics when `buffer` or the frame's damage does not match the renderer's
/// size. See [`XrgbRenderer::render_into`].
pub(crate) fn draw_direct(
    renderer: &mut XrgbRenderer,
    frame: &FrameOutput<'_>,
    age: u8,
    history: &DamageHistory,
    region: &mut Vec<PixelRect>,
    buffer: &mut [u32],
) -> bool {
    let known = missing_region(age, frame.damage, frame.full, history, region);
    if !known {
        region.push(PixelRect {
            x: 0,
            y: 0,
            width: renderer.width(),
            height: renderer.height(),
        });
    }
    renderer.render_into(
        &FrameOutput {
            damage: region,
            ..*frame
        },
        buffer,
    );
    known && !frame.damage.is_empty()
}

/// Blends the damage overlay's tint over the frame's damage rectangles.
///
/// `buffer` holds one `0x00RRGGBB` entry per pixel with `surface_width` entries
/// per row, as [`copy_rect`] writes it. Only the pixels inside `rects` change.
/// The tint lives in the window buffer alone: the next present copies the whole
/// pixmap again, so no tint reaches the renderer's pixmap and none survives in
/// a reused window buffer.
///
/// # Panics
///
/// Panics when a rect reaches outside the surface, which means the frame was
/// produced for a surface of a different size. That is a broken invariant of
/// the caller, not input from outside the library.
pub(crate) fn tint_rects(buffer: &mut [u32], surface_width: u32, rects: &[PixelRect]) {
    for rect in rects {
        let row_width = rect.width as usize;
        for y in rect.y..rect.y + rect.height {
            let row =
                &mut buffer[y as usize * surface_width as usize + rect.x as usize..][..row_width];
            for pixel in row {
                let red = tinted(*pixel >> 16 & 0xff, TINT_RGB[0]);
                let green = tinted(*pixel >> 8 & 0xff, TINT_RGB[1]);
                let blue = tinted(*pixel & 0xff, TINT_RGB[2]);
                *pixel = red << 16 | green << 8 | blue;
            }
        }
    }
}

/// The color the damage overlay tints with, as 8-bit sRGB channels.
const TINT_RGB: [u8; 3] = [255, 64, 64];

/// How strongly the damage overlay tints, out of 255.
const TINT_ALPHA: u32 = 96;

/// Blends one color channel toward the tint by the tint's opacity.
fn tinted(channel: u32, tint: u8) -> u32 {
    (channel * (255 - TINT_ALPHA) + u32::from(tint) * TINT_ALPHA + 127) / 255
}

/// The damage of the last few frames.
///
/// A buffer age of `N` means the window buffer holds the frame from `N`
/// presents ago. The pixels that differ between that frame and the current one
/// are exactly the damage of the current frame together with the damage of the
/// previous `N - 1` frames, so a present that copies that union updates only
/// the pixels the buffer is missing. The ring holds the frames an age of 1 to 3
/// can ask about and reuses the vectors between frames.
pub(crate) struct DamageHistory {
    /// The recorded frames, most recent first.
    entries: [Entry; HISTORY],
    /// How many entries hold a presented frame.
    filled: usize,
}

/// The recorded damage of one past frame.
struct Entry {
    /// The rectangles the frame changed.
    damage: Vec<PixelRect>,
    /// Whether the frame repainted the whole surface.
    full: bool,
}

impl Default for DamageHistory {
    fn default() -> DamageHistory {
        DamageHistory {
            entries: [const {
                Entry {
                    damage: Vec::new(),
                    full: false,
                }
            }; HISTORY],
            filled: 0,
        }
    }
}

impl DamageHistory {
    /// Records a presented frame and drops the oldest entry.
    pub(crate) fn push(&mut self, damage: &[PixelRect], full: bool) {
        self.entries.rotate_right(1);
        let entry = &mut self.entries[0];
        entry.damage.clear();
        entry.damage.extend_from_slice(damage);
        entry.full = full;
        self.filled = (self.filled + 1).min(HISTORY);
    }

    /// Forgets every recorded frame, keeping the vectors for reuse.
    pub(crate) fn clear(&mut self) {
        self.filled = 0;
    }
}

/// Whether the frame the last redraw rendered must skip its present.
///
/// A frame with no damage while the app runs a tick changed nothing, and the
/// window still shows what the last present left, so a present would copy the
/// whole surface only to write the same pixels again. A frame with no damage
/// without a tick is how the window system asks for a redraw, for example
/// after an X11 window was uncovered, and the full present it gets restores
/// the window's contents. While the damage overlay is on, no present is
/// skipped, so the previous frame's tint never outlives that frame.
pub(crate) fn present_skipped(damage: &[PixelRect], has_tick: bool, show_damage: bool) -> bool {
    damage.is_empty() && has_tick && !show_damage
}

/// Chooses the region a present of `age` has to draw or copy, writing it to
/// `out`.
///
/// `current` is the damage of the frame being presented and `current_full`
/// says whether it repainted the whole surface. Returns `true` when `out`
/// holds the rectangles to draw or copy and to present with damage, and
/// `false` when the whole surface must be drawn or copied and presented
/// instead. The whole-surface cases are those of [`missing_region`] and a
/// current frame with no damage.
///
/// A frame with no damage is mostly a redraw the window system asked for,
/// such as an X11 window being shown again after it was minimized or covered.
/// Without a compositor such a window has lost its contents, while the
/// buffer age still reports the buffer as current, so only a full present
/// restores it.
pub(crate) fn damage_region(
    age: u8,
    current: &[PixelRect],
    current_full: bool,
    history: &DamageHistory,
    out: &mut Vec<PixelRect>,
) -> bool {
    out.clear();
    !current.is_empty() && missing_region(age, current, current_full, history, out)
}

/// Writes to `out` the rectangles a window buffer of `age` is missing, and
/// returns whether its contents are known well enough to tell.
///
/// The region is the damage of the current frame together with the damage
/// of the frames the buffer missed. It is unknown, and the result `false`,
/// for an age outside 1 to 3, a full repaint of the current frame, a full
/// repaint within the frames the region covers, or a history that does not
/// hold the frame the buffer shows.
///
/// A past rectangle that overlaps a rectangle already in `out` is merged into
/// it when their union holds no more pixels than the two hold together, which
/// includes a past rectangle the kept one contains. A region that changes on
/// every frame is then drawn or copied once rather than once per frame the
/// buffer missed. Measured on a 1270 by 1388 window at age 2, a frame whose
/// damage covered the whole surface copied 14.1 MB and took 1525
/// microseconds to present, against 716 microseconds for a full present.
/// Drawing pays for every rectangle as well as for its pixels, so merging
/// the overlapping strips of the scroll benchmark scene took its median
/// raster at age 2 from 247 to 187 microseconds. Other rectangles are kept
/// apart, and writing a pixel twice changes nothing.
pub(crate) fn missing_region(
    age: u8,
    current: &[PixelRect],
    current_full: bool,
    history: &DamageHistory,
    out: &mut Vec<PixelRect>,
) -> bool {
    out.clear();
    if !(1..=HISTORY as u8).contains(&age) || current_full {
        return false;
    }
    let previous = age as usize - 1;
    // The buffer shows a recorded frame only when at least `age` frames were
    // presented since the history was last cleared.
    if age as usize > history.filled || history.entries[..previous].iter().any(|entry| entry.full) {
        return false;
    }
    out.extend_from_slice(current);
    for entry in &history.entries[..previous] {
        for &rect in &entry.damage {
            let overlapping = out.iter_mut().find(|kept| {
                kept.intersect(rect).is_some()
                    && kept.union(rect).area() <= kept.area() + rect.area()
            });
            match overlapping {
                Some(kept) => *kept = kept.union(rect),
                None => out.push(rect),
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "present_tests.rs"]
mod tests;
