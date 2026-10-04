//! The client side of a connection.

use crate::helpers::{receive_packet, send_packet};
use crate::server::{InternalServerPacket, ServerPacket};
use crate::token::Token;
use crate::traits::ClientName;
use crate::traits::Packet;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

/// A connected client.
///
/// `C` is the type of packets sent by the client, `S` the type of packets
/// received from the server, and `N` the type used to identify the client.
///
/// A `Client` is created with [`Client::connect`].
pub struct Client<C, S, N> {
    sockwrite: Arc<Mutex<OwnedWriteHalf>>,
    sockread: Arc<Mutex<OwnedReadHalf>>,

    phantom: PhantomData<C>,
    phantom2: PhantomData<S>,
    phantom3: PhantomData<N>,
}

impl<C, S, N> Client<C, S, N>
where
    C: Packet,
    S: Packet,
    N: ClientName,
{
    /// Connects to the server at `addr` and completes the authentication
    /// handshake using `token`.
    ///
    /// The `client_name` is sent to the server, which rejects the connection if
    /// another client with the same name is already connected.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CouldntBind`] if the connection cannot be established,
    /// or [`Error::NoResponse`] if the server does not confirm the handshake.
    pub async fn connect(client_name: N, addr: &str, token: Token) -> Result<Self> {
        log::debug!("Trying to connect to {addr} server...");
        let mut socket = TcpStream::connect(addr).await.map_err(Error::CouldntBind)?;

        let internal_packet = InternalClientPacket::ConnectRequest { token, client_name };
        let packet: ClientPacket<N, C> = ClientPacket::Internal(internal_packet);
        send_packet(&mut socket, &packet)
            .await
            .ok_or(Error::NoResponse)?;

        let received_packet: Option<ServerPacket<S>> = receive_packet(&mut socket).await;
        if !is_connect_confirm(received_packet.as_ref()) {
            return Err(Error::NoResponse);
        }

        log::info!("Connected to {addr}");

        let (sockread, sockwrite) = socket.into_split();

        Ok(Self {
            sockwrite: Arc::new(Mutex::new(sockwrite)),
            sockread: Arc::new(Mutex::new(sockread)),

            phantom: PhantomData,
            phantom2: PhantomData,
            phantom3: PhantomData,
        })
    }

    // TODO: should take network mask(?) and token, try to connect to all ip addresses
    // pub async fn connect_with_first_local(client_name: N, server_port: &str, token: Token) {
    //     let interface = blinkscan::get_default_interface().unwrap();
    //     let network = blinkscan::create_network(&interface);
    //
    //     for host in blinkscan::scan_network(network, Duration::from_secs(3)) {
    //     }
    // }

    /// Sends a packet to the server.
    ///
    /// The write is best-effort and any I/O failure is discarded.
    pub async fn send_packet(&self, packet: C) {
        let packet = ClientPacket::Data(packet);
        self.send_packet_guarded(&packet).await;
    }

    async fn send_packet_internal(&self, packet: InternalClientPacket<N>) {
        let packet = ClientPacket::Internal(packet);
        self.send_packet_guarded(&packet).await;
    }

    async fn send_packet_guarded(&self, packet: &ClientPacket<N, C>) {
        send_packet(&mut (*(self.sockwrite.lock().await)), packet).await;
    }

    /// Receives the next packet from the server.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Disconnected`] if the connection is closed or a
    /// non-data control packet is received.
    pub async fn receive_packet(&self) -> Result<S> {
        let socket = &mut (*(self.sockread.lock().await));
        let packet: Option<ServerPacket<S>> = receive_packet(socket).await;

        match packet {
            Some(ServerPacket::Data(packet)) => Ok(packet),
            Some(ServerPacket::Internal(_)) | None => Err(Error::Disconnected),
        }
    }

    /// Notifies the server that the client is disconnecting and shuts down the
    /// write half of the connection.
    pub async fn disconnect(self) {
        let internal_packet = InternalClientPacket::Disconnect;
        self.send_packet_internal(internal_packet).await;

        let _ = self.sockwrite.lock().await.shutdown().await;
    }
}

const fn is_connect_confirm<S: Packet>(packet: Option<&ServerPacket<S>>) -> bool {
    matches!(
        packet,
        Some(ServerPacket::Internal(InternalServerPacket::ConnectConfirm))
    )
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) enum ClientPacket<N, P> {
    Data(P),
    Internal(InternalClientPacket<N>),
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) enum InternalClientPacket<N> {
    ConnectRequest { token: Token, client_name: N },
    Disconnect,
}

/// Errors that can occur on the client side.
#[derive(Error, Debug)]
pub enum Error {
    /// The connection to the server could not be established.
    #[error("could not bind to addr")]
    CouldntBind(#[source] tokio::io::Error),
    /// The server did not confirm the connection handshake.
    #[error("server does not respond")]
    NoResponse,
    /// The connection to the server has been closed.
    #[error("the client has been disconnected")]
    Disconnected,
}

/// A specialized [`Result`](std::result::Result) type for client operations.
pub type Result<T> = std::result::Result<T, Error>;
