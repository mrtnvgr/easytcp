//! Low-level packet transport errors shared by the client and server.

use thiserror::Error;

/// Errors that can occur while transferring a framed packet.
#[derive(Error, Debug)]
pub(crate) enum TransportError {
    /// An I/O error occurred while reading or writing a packet.
    #[error("packet I/O error")]
    Io(#[from] tokio::io::Error),
    /// The connection was closed in the middle of a frame.
    #[error("the connection has been disconnected")]
    Disconnected,
    /// A packet exceeded the maximum allowed frame size.
    #[error("packet exceeds the maximum frame size")]
    PacketTooLarge,
    /// A packet could not be encoded or decoded.
    #[error("could not encode or decode packet")]
    Codec(#[from] postcard::Error),
}
