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
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::task::AbortHandle;

// C - client packets
// S - server packets
// N - client name
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
                        connected_clients.write().await.insert(name.clone());

                        let name_cloned = Arc::clone(&name);
                        let tx_cloned = tx.clone();
                        let server_cloned = server.clone();
                        let read_task = tokio::spawn(async move {
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

    pub async fn send_packet(&self, client_name: &N, packet: S) -> Result<()> {
        let packet = ServerPacket::Data(packet);
        self.send_packet_raw(client_name, packet).await
    }

    async fn send_packet_raw(&self, client_name: &N, packet: ServerPacket<S>) -> Result<()> {
        // Avoid race conditions
        if !(self.is_connected(client_name).await) {
            return Err(Error::NoSuchClient);
        }

        if let Some(mut socket) = self.write_sockets.get_async(client_name).await {
            if send_packet(socket.get_mut(), &packet).await.is_none() {
                self.disconnect(client_name).await?;
            }
        } else {
            return Err(Error::NoSuchClient);
        }

        Ok(())
    }

    pub async fn receive_packet(&self) -> Option<(Arc<N>, C)> {
        self.rx.lock().await.recv().await
    }

    pub async fn send_packet_to_everyone(&self, packet: S) -> Result<()> {
        for client in self.connected_clients.read().await.iter() {
            self.send_packet(client, packet.clone()).await?;
        }

        Ok(())
    }

    pub async fn is_connected(&self, client_name: &N) -> bool {
        self.connected_clients.read().await.contains(client_name)
    }

    pub async fn disconnect(&self, client_name: &N) -> Result<()> {
        self.connected_clients
            .write()
            .await
            .remove(client_name)
            .then_some(())
            .ok_or(Error::NoSuchClient)?;

        self.write_sockets.remove_async(client_name).await;

        let read_task = self.read_tasks.get(client_name).unwrap();
        read_task.abort();

        Ok(())
    }

    pub async fn disconnect_everyone(self) {
        for client in self.connected_clients.write().await.iter() {
            let _ = self.disconnect(client).await;
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

#[derive(Error, Debug)]
pub enum Error {
    #[error("could not bind to addr")]
    CouldntBind(#[source] tokio::io::Error),
    #[error("no such client connected")]
    NoSuchClient,
}

pub type Result<T> = std::result::Result<T, Error>;
