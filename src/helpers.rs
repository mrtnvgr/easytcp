use crate::traits::Packet;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

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
    let size = socket.read_u32().await.ok()?;

    let mut data = vec![0_u8; size as usize];
    socket.read_exact(&mut data).await.ok()?;

    postcard::from_bytes(&data).ok()
}

fn to_bytes<P: Packet>(packet: &P) -> Option<Vec<u8>> {
    let packet = postcard::to_stdvec(packet).ok()?;
    let length = u32::try_from(packet.len()).ok()?;

    let mut data = length.to_be_bytes().to_vec();
    data.extend(packet.as_slice());

    Some(data)
}
