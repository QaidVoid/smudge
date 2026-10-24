//! Copying the renderer's surface into the window buffer, and choosing how
//! much of the surface a present has to carry.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use smudge_core::geometry::PixelRect;

/// How many past frames the damage ring keeps.
///
/// Softbuffer reports ages of at most 3 for the double and triple buffered
/// backends this library targets, so a longer ring could never be read.
const HISTORY: usize = 3;

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
    /// 40,000 bytes.
    pub bytes_copied: u64,
    /// The width of the presented surface in physical pixels.
    pub surface_width: u32,
    /// The height of the presented surface in physical pixels.
    pub surface_height: u32,
    /// How long the renderer took to paint this frame's damage into its
    /// surface.
    pub raster: Duration,
    /// How long the present took, from taking the window buffer to handing it
    /// back to the platform.
    pub present: Duration,
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
}

/// Chooses the region a present of `age` has to copy, writing it to `out`.
///
/// `current` is the damage of the frame being presented and `current_full`
/// says whether it repainted the whole surface. Returns `true` when `out`
/// holds the rectangles to copy and to present with damage, and `false` when
/// the whole pixmap must be copied and presented instead. The whole-pixmap
/// cases are an age outside 1 to 3, a full repaint of the current frame, a
/// current frame with no damage, a full repaint within the frames the region
/// covers, or history that does not reach back far enough.
///
/// A past rectangle that a rectangle already in `out` contains is skipped, so a
/// region that changes on every frame is copied once rather than once per
/// frame the buffer missed. Measured on a 1270 by 1388 window at age 2, a frame
/// whose damage covered the whole surface copied 14.1 MB and took 1525
/// microseconds to present, against 716 microseconds for a full present.
/// Rectangles that only partly overlap are both kept, and writing a pixel twice
/// changes nothing.
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
    if !(1..=HISTORY as u8).contains(&age) || current_full || current.is_empty() {
        return false;
    }
    let previous = age as usize - 1;
    if previous > history.filled || history.entries[..previous].iter().any(|entry| entry.full) {
        return false;
    }
    out.extend_from_slice(current);
    for entry in &history.entries[..previous] {
        for &rect in &entry.damage {
            if !out.iter().any(|kept| kept.contains_rect(rect)) {
                out.push(rect);
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "present_tests.rs"]
mod tests;
