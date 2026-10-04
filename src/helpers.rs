use crate::traits::Packet;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// Maximum accepted size, in bytes, of a single framed packet payload.
///
/// Frames larger than this are rejected before any buffer is allocated.
const MAX_PACKET_SIZE: usize = 8 * 1024 * 1024;

/// Returns whether the packet was delivered successfully
pub async fn send_packet<P: Packet, S: AsyncWriteExt + Unpin + Send>(
    socket: &mut S,
    packet: &P,
) -> Option<()> {
    let data = to_bytes(packet)?;
    socket.write_all(data.as_slice()).await.ok()?;
    socket.flush().await.ok()?;
    Some(())
}

pub async fn receive_packet<P: Packet, S: AsyncRead + Unpin + Send>(socket: &mut S) -> Option<P> {
    let size = usize::try_from(socket.read_u32().await.ok()?).ok()?;

    if size > MAX_PACKET_SIZE {
        return None;
    }

    let mut data = vec![0_u8; size];
    socket.read_exact(&mut data).await.ok()?;

    let (packet, remainder) = postcard::take_from_bytes::<P>(&data).ok()?;

    if !remainder.is_empty() {
        return None;
    }

    Some(packet)
}

fn to_bytes<P: Packet>(packet: &P) -> Option<Vec<u8>> {
    let packet = postcard::to_stdvec(packet).ok()?;

    if packet.len() > MAX_PACKET_SIZE {
        return None;
    }

    let length = u32::try_from(packet.len()).ok()?;

    let mut data = length.to_be_bytes().to_vec();
    data.extend(packet.as_slice());

    Some(data)
}
