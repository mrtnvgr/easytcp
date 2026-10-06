use easytcp::client::Client;
use easytcp::server::Server;
use easytcp::token::Token;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct ClientMsg(u32);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct ServerMsg(u32);

type TestServer = Server<ClientMsg, ServerMsg, String>;
type TestClient = Client<ClientMsg, ServerMsg, String>;

async fn start_server() -> (Arc<TestServer>, Token, SocketAddr) {
    let token = Token::new("correct horse battery staple");
    let server = Arc::new(TestServer::new());
    let addr = server
        .clone()
        .start("127.0.0.1:0", token.clone())
        .await
        .expect("server should bind");

    (server, token, addr)
}

fn name(value: &str) -> String {
    value.to_string()
}

#[tokio::test]
async fn handshake_and_roundtrip() {
    let (server, token, addr) = start_server().await;

    let client = TestClient::connect(name("alice"), &addr.to_string(), token)
        .await
        .unwrap();

    client.send_packet(ClientMsg(1)).await.unwrap();

    let (client_name, packet) = server.receive_packet().await.unwrap();
    assert_eq!(*client_name, "alice");
    assert_eq!(packet, ClientMsg(1));

    server
        .send_packet(&client_name, ServerMsg(2))
        .await
        .unwrap();
    assert_eq!(client.receive_packet().await.unwrap(), ServerMsg(2));

    server.shutdown().await;
}

#[tokio::test]
async fn rejects_wrong_token() {
    let (server, _token, addr) = start_server().await;

    let wrong = Token::new("not the token");
    let result = TestClient::connect(name("bob"), &addr.to_string(), wrong).await;

    assert!(result.is_err());

    server.shutdown().await;
}

#[tokio::test]
async fn broadcasts_to_every_client() {
    let (server, token, addr) = start_server().await;

    let first = TestClient::connect(name("a"), &addr.to_string(), token.clone())
        .await
        .unwrap();
    let second = TestClient::connect(name("b"), &addr.to_string(), token)
        .await
        .unwrap();

    let results = server.send_packet_to_everyone(ServerMsg(9)).await;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|(_, result)| result.is_ok()));

    assert_eq!(first.receive_packet().await.unwrap(), ServerMsg(9));
    assert_eq!(second.receive_packet().await.unwrap(), ServerMsg(9));

    server.shutdown().await;
}

#[tokio::test]
async fn duplicate_name_is_rejected() {
    let (server, token, addr) = start_server().await;

    let _first = TestClient::connect(name("dup"), &addr.to_string(), token.clone())
        .await
        .unwrap();
    let second = TestClient::connect(name("dup"), &addr.to_string(), token).await;

    assert!(second.is_err());
    assert_eq!(server.clients().len(), 1);

    server.shutdown().await;
}

#[tokio::test]
async fn server_disconnect_closes_client() {
    let (server, token, addr) = start_server().await;

    let client = TestClient::connect(name("x"), &addr.to_string(), token)
        .await
        .unwrap();

    server.disconnect(&name("x")).await.unwrap();
    assert!(!server.is_connected(&name("x")).await);

    assert!(client.receive_packet().await.is_err());

    server.shutdown().await;
}

#[tokio::test]
async fn client_disconnect_is_observed_by_server() {
    let (server, token, addr) = start_server().await;

    let client = TestClient::connect(name("y"), &addr.to_string(), token)
        .await
        .unwrap();

    client.disconnect().await;

    let mut removed = false;
    for _ in 0..200 {
        if !server.is_connected(&name("y")).await {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(removed, "server should observe the client disconnect");

    server.shutdown().await;
}
