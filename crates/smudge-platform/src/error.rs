//! The error a window run returns.

use std::error::Error;
use std::fmt;

use softbuffer::SoftBufferError;
use winit::error::{EventLoopError, OsError};

/// Something that stopped a window from opening, running or presenting a
/// frame.
#[derive(Debug)]
pub enum PlatformError {
    /// The event loop could not be created or run to completion.
    EventLoop(EventLoopError),
    /// The operating system refused to create the window.
    Window(OsError),
    /// The software presentation surface could not be created, resized or
    /// presented.
    Surface(SoftBufferError),
    /// The window options gave a clear color that is not opaque.
    ///
    /// Presentation copies only the color channels of the surface into the
    /// window, so a clear color with any transparency cannot be shown.
    ClearColor,
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlatformError::EventLoop(error) => write!(formatter, "the event loop failed: {error}"),
            PlatformError::Window(error) => {
                write!(formatter, "the window could not be created: {error}")
            }
            PlatformError::Surface(error) => {
                write!(formatter, "the presentation surface failed: {error}")
            }
            PlatformError::ClearColor => write!(
                formatter,
                "the clear color must be opaque, because presentation drops \
                 the alpha channel of the surface"
            ),
        }
    }
}

impl Error for PlatformError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PlatformError::EventLoop(error) => Some(error),
            PlatformError::Window(error) => Some(error),
            PlatformError::Surface(error) => Some(error),
            PlatformError::ClearColor => None,
        }
    }
}
