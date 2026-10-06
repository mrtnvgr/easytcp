use easytcp::server::Server;
use easytcp::token::Token;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Request(String);

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Response(String);

type EchoServer = Server<Request, Response, String>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = Token::new("secret");

    let server = Arc::new(EchoServer::new());
    let addr = server.clone().start("127.0.0.1:4321", token).await?;
    println!("Echo server listening on {addr}");

    while let Some((client, Request(message))) = server.receive_packet().await {
        println!("{client:?} says: {message}");

        if let Err(error) = server.send_packet(&client, Response(message)).await {
            eprintln!("Failed to reply to {client:?}: {error}");
        }
    }

    Ok(())
}
