use easytcp::client::Client;
use easytcp::token::Token;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Request(String);

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Response(String);

type EchoClient = Client<Request, Response, String>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = Token::new("secret");

    let client = EchoClient::connect("guest".to_string(), "127.0.0.1:4321", token).await?;
    println!("Connected to the server");

    client.send_packet(Request("hello".to_string())).await?;

    let Response(reply) = client.receive_packet().await?;
    println!("Server replied: {reply}");

    client.disconnect().await;

    Ok(())
}
