//! The server side of a connection.

use crate::client::{ClientPacket, InternalClientPacket};
use crate::helpers::{receive_packet, send_packet};
use crate::token::Token;
use crate::traits::ClientName;
use crate::traits::Packet;
use scc::HashMap;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::AbortHandle;

/// A single connected client's write half and its background read task.
struct ClientEntry {
    write_socket: Arc<Mutex<OwnedWriteHalf>>,
    read_task: AbortHandle,
}

/// A TCP server that manages authenticated clients.
///
/// `C` is the type of packets received from clients, `S` the type of packets
/// sent to clients, and `N` the type used to identify clients.
///
/// Create a server with [`Server::new`] and start accepting connections with
/// [`Server::start`].
pub struct Server<C, S, N> {
    clients: HashMap<Arc<N>, ClientEntry>,

    tx: mpsc::Sender<(Arc<N>, C)>,
    rx: Mutex<mpsc::Receiver<(Arc<N>, C)>>,

    abort_handle: Mutex<Option<AbortHandle>>,

    phantom: PhantomData<C>,
    phantom2: PhantomData<S>,
}

impl<C, S, N> Server<C, S, N>
where
    C: Packet,
    S: Packet,
    N: ClientName,
{
    /// Creates a new server that is not yet accepting connections.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);

        Self {
            clients: HashMap::new(),

            tx,
            rx: Mutex::new(rx),

            abort_handle: Mutex::new(None),

            phantom: PhantomData,
            phantom2: PhantomData,
        }
    }

    /// Binds to `addr` and starts accepting client connections in the
    /// background.
    ///
    /// Connections are authenticated against `token`. This method returns once
    /// the listener is bound; incoming connections are handled by a spawned
    /// task.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Bind`] if the address cannot be bound.
    pub async fn start(self: Arc<Self>, addr: &str, token: Token) -> Result<()> {
        log::trace!("Server started on {addr}");
        let listener = TcpListener::bind(addr).await.map_err(Error::Bind)?;

        let token = Arc::new(token);

        let server = Arc::clone(&self);
        let abort_handle = tokio::spawn(async move {
            loop {
                let (socket, _) = match listener.accept().await {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        log::error!("Failed to accept a connection: {error}");
                        continue;
                    }
                };

                match Self::handshake(&token, socket).await {
                    Some((name, socket)) => {
                        Self::register(Arc::clone(&server), name, socket).await;
                    }
                    None => log::error!("A client failed to connect"),
                }
            }
        })
        .abort_handle();

        *(self.abort_handle.lock().await) = Some(abort_handle);

        Ok(())
    }

    /// Registers a freshly handshaken client and spawns its read task.
    ///
    /// Duplicate names are rejected: the existing client is left untouched and
    /// the new connection is dropped.
    async fn register(server: Arc<Self>, name: N, socket: TcpStream) {
        let (mut sockread, sockwrite) = socket.into_split();

        let name = Arc::new(name);

        // The read task must not start reading until the client has been
        // inserted into the registry. Otherwise a client that closes
        // immediately can be cleaned up by the task before registration
        // completes, leaving a stale entry.
        let ready = Arc::new(Notify::new());

        let name_cloned = Arc::clone(&name);
        let tx_cloned = server.tx.clone();
        let server_cloned = Arc::clone(&server);
        let ready_cloned = Arc::clone(&ready);
        let read_task = tokio::spawn(async move {
            ready_cloned.notified().await;

            loop {
                let packet: Option<ClientPacket<N, C>> = receive_packet(&mut sockread).await.ok();

                let _ = match packet {
                    Some(ClientPacket::Data(packet)) => {
                        // TODO: do not clone names
                        tx_cloned.send((name_cloned.clone(), packet)).await
                    }
                    _ => break,
                };
            }

            let _ = server_cloned.disconnect(&name_cloned).await;
            log::info!("Client {name_cloned:?} has been disconnected");
        })
        .abort_handle();

        let entry = ClientEntry {
            write_socket: Arc::new(Mutex::new(sockwrite)),
            read_task: read_task.clone(),
        };

        if server
            .clients
            .insert_async(name.clone(), entry)
            .await
            .is_err()
        {
            read_task.abort();
            log::error!("Client {name:?} has already connected");
            return;
        }

        ready.notify_one();
        log::info!("Client {name:?} has connected");
    }

    /// Stops accepting new connections and disconnects every connected client.
    ///
    /// This aborts the background accept loop, which releases the server's
    /// internal `Arc<Self>` so the server can be dropped normally afterwards.
    /// Calling `shutdown` on a server that was never started is a no-op.
    /// TODO(some-time-later-for-now-ignore): shutdown should consume the server
    pub async fn shutdown(&self) {
        if let Some(abort_handle) = self.abort_handle.lock().await.take() {
            abort_handle.abort();
        }

        self.disconnect_everyone().await;
    }

    /// Sends a packet to the client identified by `client_name`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoSuchClient`] if no client with that name is connected
    /// or if the client is disconnected while sending.
    pub async fn send_packet(&self, client_name: &N, packet: S) -> Result<()> {
        let packet = ServerPacket::Data(packet);
        self.send_packet_raw(client_name, packet).await
    }

    async fn send_packet_raw(&self, client_name: &N, packet: ServerPacket<S>) -> Result<()> {
        // Clone the write half out of the map so no `scc` bucket guard is held
        // while awaiting the socket write.
        let write_socket = self
            .clients
            .read_async(client_name, |_, entry| Arc::clone(&entry.write_socket))
            .await
            .ok_or(Error::NoSuchClient)?;

        let failed = send_packet(&mut *write_socket.lock().await, &packet)
            .await
            .is_err();

        if failed {
            self.disconnect(client_name).await?;
        }

        Ok(())
    }

    /// Receives the next packet sent by any client.
    ///
    /// Returns `None` once every sender has been dropped.
    pub async fn receive_packet(&self) -> Option<(Arc<N>, C)> {
        self.rx.lock().await.recv().await
    }

    /// Sends `packet` to every connected client.
    ///
    /// Every client is attempted even if some fail. The returned vector holds
    /// one `(client_name, result)` entry per client that was connected when the
    /// call started, in an arbitrary order.
    pub async fn send_packet_to_everyone(&self, packet: S) -> Vec<(Arc<N>, Result<()>)> {
        let clients = self.clients().await;

        let mut results = Vec::with_capacity(clients.len());

        for client in clients {
            let result = self.send_packet(&client, packet.clone()).await;
            results.push((client, result));
        }

        results
    }

    /// Returns whether a client with the given name is currently connected.
    #[must_use]
    pub async fn is_connected(&self, client_name: &N) -> bool {
        self.clients.contains_async(client_name).await
    }

    /// Disconnects the client identified by `client_name`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoSuchClient`] if no client with that name is
    /// connected.
    pub async fn disconnect(&self, client_name: &N) -> Result<()> {
        let Some((_, entry)) = self.clients.remove_async(client_name).await else {
            return Err(Error::NoSuchClient);
        };

        entry.read_task.abort();

        Ok(())
    }

    /// Disconnects every connected client.
    pub async fn disconnect_everyone(&self) {
        for client in self.clients().await {
            let _ = self.disconnect(&client).await;
        }
    }

    /// Returns the names of all currently connected clients.
    ///
    /// The order of the returned names is arbitrary.
    #[must_use]
    pub async fn clients(&self) -> Vec<Arc<N>> {
        let mut names = Vec::new();

        self.clients
            .scan_async(|name, _| names.push(Arc::clone(name)))
            .await;

        names
    }

    async fn handshake(server_token: &Arc<Token>, mut socket: TcpStream) -> Option<(N, TcpStream)> {
        let packet: ClientPacket<N, C> = receive_packet(&mut socket).await.ok()?;

        match packet {
            ClientPacket::Internal(InternalClientPacket::ConnectRequest { token, client_name })
                if **server_token == token =>
            {
                let internal_packet = InternalServerPacket::ConnectConfirm;
                let packet: ServerPacket<S> = ServerPacket::Internal(internal_packet);

                let response = send_packet(&mut socket, &packet).await;
                let connected_client = Some((client_name, socket));
                response.ok().and(connected_client)
            }
            _ => None,
        }
    }
}

impl<C, S, N> Default for Server<C, S, N>
where
    C: Packet,
    S: Packet,
    N: ClientName,
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) enum ServerPacket<P> {
    Data(P),
    Internal(InternalServerPacket),
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) enum InternalServerPacket {
    ConnectConfirm,
}

/// Errors that can occur on the server side.
#[derive(Error, Debug)]
pub enum Error {
    /// The listener could not bind to the requested address.
    #[error("could not bind to address")]
    Bind(#[source] tokio::io::Error),
    /// The requested client is not connected.
    #[error("no such client connected")]
    NoSuchClient,
}

/// A specialized [`Result`](std::result::Result) type for server operations.
pub type Result<T> = std::result::Result<T, Error>;
