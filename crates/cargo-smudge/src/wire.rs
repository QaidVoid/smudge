//! The frames the builder and the app exchange over their socket, which
//! `HOT_SOCKET_ENVIRONMENT` documents in the platform crate.

use std::io::{self, Read, Write};

/// The kind of the frame that introduces the app.
pub(crate) const HELLO: u8 = 1;

/// The kind of the frame that carries a jump table.
pub(crate) const PATCH: u8 = 2;

/// The kind of the frame that reports a patch as applied.
pub(crate) const APPLIED: u8 = 3;

/// The kind of the frame that reports a patch as refused.
pub(crate) const FAILED: u8 = 4;

/// The kind of the frame that tells the app a patch is being built.
pub(crate) const BUILDING: u8 = 5;

/// The kind of the frame that tells the app a patch could not be built,
/// with the compiler's message.
pub(crate) const BUILD_FAILED: u8 = 6;

/// The largest payload the builder accepts, which mirrors the limit the
/// receiver applies to its own reads.
const MAX_PAYLOAD: u32 = 64 << 20;

/// Reads one frame, or answers `None` when the stream ended: cleanly
/// between frames, or mid-header, which no caller can tell apart from a
/// closed connection and treats the same way.
///
/// # Errors
///
/// Returns why the frame could not be read.
pub(crate) fn read_frame(stream: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0; 5];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let [kind, length @ ..] = header;
    let length = u32::from_le_bytes(length);
    if length > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a frame announces {length} bytes, over the limit of {MAX_PAYLOAD}"),
        ));
    }
    let mut payload = vec![0; length as usize];
    stream.read_exact(&mut payload)?;
    Ok(Some((kind, payload)))
}

/// Writes one frame of `kind` carrying `payload`.
///
/// # Errors
///
/// Returns why the frame could not be written.
pub(crate) fn write_frame(stream: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the payload is too long"))?;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}
