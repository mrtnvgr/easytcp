use crate::error::TransportError;
use crate::traits::Packet;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ErrorKind};

type Result<T> = std::result::Result<T, TransportError>;

/// Maximum accepted size, in bytes, of a single framed packet payload.
///
/// Frames larger than this are rejected before any buffer is allocated.
const MAX_PACKET_SIZE: usize = 8 * 1024 * 1024;

/// Sends a single framed packet over `socket`.
///
/// # Errors
///
/// Returns [`TransportError::PacketTooLarge`] if the encoded packet exceeds the
/// maximum frame size, [`TransportError::Codec`] if it cannot be encoded, or
/// [`TransportError::Io`] if the write fails.
pub async fn send_packet<P: Packet, S: AsyncWriteExt + Unpin + Send>(
    socket: &mut S,
    packet: &P,
) -> Result<()> {
    let data = to_bytes(packet)?;
    socket.write_all(data.as_slice()).await?;
    socket.flush().await?;
    Ok(())
}

/// Receives a single framed packet from `socket`.
///
/// # Errors
///
/// Returns [`TransportError::Disconnected`] if the stream ends mid-frame,
/// [`TransportError::PacketTooLarge`] if the frame length exceeds the maximum,
/// [`TransportError::Codec`] if the payload cannot be decoded, or
/// [`TransportError::Io`] if the read fails.
pub async fn receive_packet<P: Packet, S: AsyncRead + Unpin + Send>(socket: &mut S) -> Result<P> {
    let size = match socket.read_u32().await {
        Ok(size) => size,
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
            return Err(TransportError::Disconnected);
        }
        Err(error) => return Err(TransportError::Io(error)),
    };

    let size = usize::try_from(size).map_err(|_| TransportError::PacketTooLarge)?;

    if size > MAX_PACKET_SIZE {
        return Err(TransportError::PacketTooLarge);
    }

    let mut data = vec![0_u8; size];

    if let Err(error) = socket.read_exact(&mut data).await {
        return Err(match error.kind() {
            ErrorKind::UnexpectedEof => TransportError::Disconnected,
            _ => TransportError::Io(error),
        });
    }

    let (packet, remainder) =
        postcard::take_from_bytes::<P>(&data).map_err(TransportError::Codec)?;

    if !remainder.is_empty() {
        return Err(TransportError::Codec(
            postcard::Error::DeserializeBadEncoding,
        ));
    }

    Ok(packet)
}

fn to_bytes<P: Packet>(packet: &P) -> Result<Vec<u8>> {
    let packet = postcard::to_stdvec(packet).map_err(TransportError::Codec)?;

    if packet.len() > MAX_PACKET_SIZE {
        return Err(TransportError::PacketTooLarge);
    }

    let length = u32::try_from(packet.len()).map_err(|_| TransportError::PacketTooLarge)?;

    let mut data = length.to_be_bytes().to_vec();
    data.extend(packet.as_slice());

    Ok(data)
}
