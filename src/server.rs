//! The server side of a connection.

use crate::client::{ClientPacket, InternalClientPacket};
use crate::helpers::{receive_packet, send_packet};
use crate::token::Token;
use crate::traits::ClientName;
use crate::traits::Packet;
use scc::HashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::Arc;
use thiserror::Error;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tokio::task::AbortHandle;

/// A TCP server that manages authenticated clients.
///
/// `C` is the type of packets received from clients, `S` the type of packets
/// sent to clients, and `N` the type used to identify clients.
///
/// Create a server with [`Server::new`] and start accepting connections with
/// [`Server::start`].
pub struct Server<C, S, N> {
    connected_clients: Arc<RwLock<HashSet<Arc<N>>>>,
    write_sockets: Arc<HashMap<Arc<N>, OwnedWriteHalf>>,
    read_tasks: Arc<HashMap<Arc<N>, AbortHandle>>,

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
        let connected_clients = Arc::new(RwLock::new(HashSet::new()));
        let write_sockets = Arc::new(HashMap::new());
        let read_tasks = Arc::new(HashMap::new());

        let (tx, rx) = mpsc::channel(32);

        Self {
            connected_clients,
            write_sockets,
            read_tasks,

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
    /// Returns [`Error::CouldntBind`] if the address cannot be bound.
    pub async fn start(self: Arc<Self>, addr: &str, token: Token) -> Result<()> {
        log::trace!("Server started on {addr}");
        let listener = TcpListener::bind(addr).await.map_err(Error::CouldntBind)?;

        let token = Arc::new(token);

        let connected_clients = Arc::clone(&self.connected_clients);
        let write_sockets = Arc::clone(&self.write_sockets);
        let read_tasks = Arc::clone(&self.read_tasks);

        let tx = self.tx.clone();

        let server = Arc::clone(&self);
        let abort_handle = tokio::spawn(async move {
            loop {
                let server = Arc::clone(&server);

                match Self::accept_client(&token, &listener).await {
                    Some((name, socket)) if !server.is_connected(&name).await => {
                        let (mut sockread, sockwrite) = socket.into_split();

                        let name = Arc::new(name);

                        let _ = write_sockets.insert_async(name.clone(), sockwrite).await;

                        // The read task must not start reading until the client
                        // is fully registered. Otherwise a client that closes
                        // immediately can be cleaned up by the task before the
                        // registration below completes, leaving a stale entry.
                        let ready = Arc::new(Notify::new());

                        let name_cloned = Arc::clone(&name);
                        let tx_cloned = tx.clone();
                        let server_cloned = server.clone();
                        let ready_cloned = Arc::clone(&ready);
                        let read_task = tokio::spawn(async move {
                            ready_cloned.notified().await;

                            loop {
                                let packet: Option<ClientPacket<N, C>> =
                                    receive_packet(&mut sockread).await;

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

                        let _ = read_tasks.insert_async(name.clone(), read_task).await;
                        connected_clients.write().await.insert(name.clone());
                        ready.notify_one();

                        log::info!("Client {name:?} has connected");
                    }
                    Some((name, _)) => log::error!("Client {name:?} has already connected"),
                    None => log::error!("A client failed to connect"),
                }
            }
        })
        .abort_handle();

        *(self.abort_handle.lock().await) = Some(abort_handle);

        Ok(())
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
        // Avoid race conditions
        if !(self.is_connected(client_name).await) {
            return Err(Error::NoSuchClient);
        }

        let Some(mut socket) = self.write_sockets.get_async(client_name).await else {
            return Err(Error::NoSuchClient);
        };

        let failed = send_packet(socket.get_mut(), &packet).await.is_none();

        // Release the scc bucket guard before `disconnect` re-enters the map.
        drop(socket);

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
        let clients: Vec<Arc<N>> = self
            .connected_clients
            .read()
            .await
            .iter()
            .cloned()
            .collect();

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
        self.connected_clients.read().await.contains(client_name)
    }

    /// Disconnects the client identified by `client_name`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoSuchClient`] if no client with that name is
    /// connected.
    pub async fn disconnect(&self, client_name: &N) -> Result<()> {
        let was_connected = self.connected_clients.write().await.remove(client_name);

        self.write_sockets.remove_async(client_name).await;

        if let Some((_, read_task)) = self.read_tasks.remove_async(client_name).await {
            read_task.abort();
        }

        was_connected.then_some(()).ok_or(Error::NoSuchClient)
    }

    /// Disconnects every connected client and consumes the server handle.
    pub async fn disconnect_everyone(self) {
        let clients: Vec<Arc<N>> = self
            .connected_clients
            .read()
            .await
            .iter()
            .cloned()
            .collect();

        for client in clients {
            let _ = self.disconnect(&client).await;
        }
    }

    async fn accept_client(
        server_token: &Arc<Token>,
        listener: &TcpListener,
    ) -> Option<(N, TcpStream)> {
        let (mut socket, _) = listener.accept().await.unwrap();

        let packet: ClientPacket<N, C> = receive_packet(&mut socket).await?;

        match packet {
            ClientPacket::Internal(InternalClientPacket::ConnectRequest { token, client_name })
                if **server_token == token =>
            {
                let internal_packet = InternalServerPacket::ConnectConfirm;
                let packet: ServerPacket<S> = ServerPacket::Internal(internal_packet);

                let response = send_packet(&mut socket, &packet).await;
                let connected_client = Some((client_name, socket));
                response.and(connected_client)
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
    #[error("could not bind to addr")]
    CouldntBind(#[source] tokio::io::Error),
    /// The requested client is not connected.
    #[error("no such client connected")]
    NoSuchClient,
}

/// A specialized [`Result`](std::result::Result) type for server operations.
pub type Result<T> = std::result::Result<T, Error>;
