# easytcp

`easytcp` is a client/server crate with typed packets.

Any type that implements serde's `Serialize`/`Deserialize` can be a packet.

Client name is a *(de)serializable* type too, this can be used to:

- Allow for any client name via `String` type.
- Allow for a limited clients using an enum. (e.g. `Id::Home`, `Id::Work`).
- ...

## Installation

```console
cargo add easytcp
```

## Getting started

Define the packet types shared by both sides:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Request(String);

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Response(String);
```

Server:

```rust
use easytcp::server::Server;
use easytcp::token::Token;
use std::sync::Arc;

type EchoServer = Server<Request, Response, String>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = Arc::new(EchoServer::new());
    let addr = server
        .clone()
        .start("127.0.0.1:4321", Token::new("secret"))
        .await?;
    println!("Echo server listening on {addr}");

    while let Some((client, Request(message))) = server.receive_packet().await {
        server.send_packet(&client, Response(message)).await?;
    }

    Ok(())
}
```

Client:

```rust
use easytcp::client::Client;
use easytcp::token::Token;

type EchoClient = Client<Request, Response, String>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = EchoClient::connect("guest".to_string(), "127.0.0.1:4321", Token::new("secret"))
        .await?;

    client.send_packet(Request("hello".to_string())).await?;
    let Response(reply) = client.receive_packet().await?;
    println!("Server replied: {reply}");

    client.disconnect().await;

    Ok(())
}
```

See the [`examples`](examples) directory for runnable versions of both.

## Projects using easytcp

- [multipad](https://github.com/mrtnvgr/multipad) - control multiple devices
  with one gamepad.
