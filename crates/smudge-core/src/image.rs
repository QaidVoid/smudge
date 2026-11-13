//! Premultiplied RGBA pixel data that widgets paint into rectangles.
//!
//! Decoding is not part of the library: an application decodes a file off the
//! UI thread and hands the pixels over. [`Image`] is `Send` and `Sync`, so a
//! background thread can hold one, and it is cheap to clone, so a signal can
//! hold one while the display list borrows it.
//!
//! The pixels are premultiplied RGBA8, row-major with no padding: each byte
//! is a channel value with the red, green and blue channels already
//! multiplied by the alpha. Premultiplied pixels blend without division. A
//! buffer that arrives straight-alpha from a decoder goes through
//! [`Image::from_straight_rgba`], which does the multiplication.

use crate::element::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The source of the identifiers every [`Image`] carries.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Premultiplied RGBA8 pixel data, shared by every element that shows it.
///
/// The pixels are row-major with no padding, four bytes per pixel in red,
/// green, blue, alpha order, with the color channels already multiplied by
/// the alpha. Two images compare equal when they came from the same
/// [`Image::from_premultiplied_rgba`] call, so a display list that re-emits
/// the same image damages nothing new.
///
/// # Examples
///
/// A two by two image, fully opaque red and blue:
///
/// ```
/// use smudge_core::image::Image;
///
/// let image = Image::from_premultiplied_rgba(
///     2,
///     2,
///     vec![
///         255, 0, 0, 255, 255, 0, 0, 255,
///         0, 0, 255, 255, 0, 0, 255, 255,
///     ],
/// )
/// .expect("the buffer holds four pixels");
/// assert_eq!(image.width(), 2);
/// ```
#[derive(Clone)]
pub struct Image {
    data: Arc<ImageData>,
}

struct ImageData {
    id: u64,
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
}

impl Image {
    /// Builds an image from premultiplied RGBA8 pixels, row-major with no
    /// padding, four bytes per pixel in red, green, blue, alpha order, with
    /// the color channels already multiplied by the alpha.
    ///
    /// A color channel above its pixel's alpha cannot come out of a
    /// multiplication by that alpha, so it is clamped to the alpha. Both
    /// renderers then blend the same valid pixel. A buffer that holds
    /// straight alpha belongs in [`Image::from_straight_rgba`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`ImageError::ZeroSize`] when `width` or `height` is zero, and
    /// [`ImageError::PixelCount`] when `pixels` does not hold exactly
    /// `width * height * 4` bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use smudge_core::image::{Image, ImageError};
    ///
    /// let error = Image::from_premultiplied_rgba(2, 2, vec![0; 7])
    ///     .expect_err("the buffer is too short");
    /// assert!(matches!(
    ///     error,
    ///     ImageError::PixelCount {
    ///         expected: 16,
    ///         found: 7,
    ///     }
    /// ));
    /// ```
    pub fn from_premultiplied_rgba(
        width: u32,
        height: u32,
        mut pixels: Vec<u8>,
    ) -> Result<Image, ImageError> {
        check_size(width, height, pixels.len())?;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            let alpha = pixel[3];
            for channel in &mut pixel[..3] {
                *channel = (*channel).min(alpha);
            }
        }
        Ok(Image::new(width, height, pixels))
    }

    /// Builds an image from straight-alpha RGBA8 pixels, the form most
    /// decoders produce, by multiplying each color channel by its pixel's
    /// alpha and rounding to the nearest value.
    ///
    /// The layout is the one [`Image::from_premultiplied_rgba`] takes: row
    /// major with no padding, four bytes per pixel in red, green, blue,
    /// alpha order. The conversion happens in place, without a second
    /// buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ImageError::ZeroSize`] when `width` or `height` is zero, and
    /// [`ImageError::PixelCount`] when `pixels` does not hold exactly
    /// `width * height * 4` bytes.
    ///
    /// # Examples
    ///
    /// White at half alpha becomes half-intensity white:
    ///
    /// ```
    /// use smudge_core::image::Image;
    ///
    /// let image = Image::from_straight_rgba(1, 1, vec![255, 255, 255, 128])
    ///     .expect("one pixel fills the buffer");
    /// assert_eq!(image.pixels(), &[128, 128, 128, 128]);
    /// ```
    pub fn from_straight_rgba(
        width: u32,
        height: u32,
        mut pixels: Vec<u8>,
    ) -> Result<Image, ImageError> {
        check_size(width, height, pixels.len())?;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        Ok(Image::new(width, height, pixels))
    }

    /// Wraps pixels that passed [`check_size`] under a fresh identifier.
    fn new(width: u32, height: u32, pixels: Vec<u8>) -> Image {
        Image {
            data: Arc::new(ImageData {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                width,
                height,
                pixels: pixels.into_boxed_slice(),
            }),
        }
    }

    /// The image's width in pixels.
    pub fn width(&self) -> u32 {
        self.data.width
    }

    /// The image's height in pixels.
    pub fn height(&self) -> u32 {
        self.data.height
    }

    /// The premultiplied RGBA8 pixels, row-major with no padding.
    pub fn pixels(&self) -> &[u8] {
        &self.data.pixels
    }

    /// The identifier that names this image's data for the lifetime of the
    /// process.
    ///
    /// Renderers use it as an upload cache key without hashing the pixels.
    pub fn id(&self) -> u64 {
        self.data.id
    }
}

impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.data.id == other.data.id
    }
}

impl From<Option<Image>> for Value<Option<Image>> {
    fn from(value: Option<Image>) -> Value<Option<Image>> {
        Value::Static(value)
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("id", &self.data.id)
            .field("width", &self.data.width)
            .field("height", &self.data.height)
            .finish()
    }
}

/// Checks that a `width` by `height` image has a pixel and that a buffer of
/// `found` bytes holds exactly its RGBA8 pixels.
fn check_size(width: u32, height: u32, found: usize) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::ZeroSize { width, height });
    }
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|count| count.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok());
    match expected {
        Some(expected) if expected == found => Ok(()),
        expected => Err(ImageError::PixelCount {
            expected: expected.unwrap_or(usize::MAX),
            found,
        }),
    }
}

/// Why an [`Image`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImageError {
    /// The width or the height is zero, so the image has no pixel.
    ZeroSize {
        /// The width asked for, in pixels.
        width: u32,
        /// The height asked for, in pixels.
        height: u32,
    },
    /// The pixel buffer's length does not match the width, height and
    /// format.
    PixelCount {
        /// The length the width and height call for, in bytes, or
        /// `usize::MAX` when that length does not fit in a `usize`.
        expected: usize,
        /// The length the buffer has, in bytes.
        found: usize,
    },
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::ZeroSize { width, height } => {
                write!(formatter, "an image cannot be {width}x{height} pixels")
            }
            ImageError::PixelCount { expected, found } => write!(
                formatter,
                "the pixel buffer holds {found} bytes, but the size calls for {expected}"
            ),
        }
    }
}

impl std::error::Error for ImageError {}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
