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
        }
    }
}

impl Error for PlatformError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PlatformError::EventLoop(error) => Some(error),
            PlatformError::Window(error) => Some(error),
            PlatformError::Surface(error) => Some(error),
        }
    }
}
