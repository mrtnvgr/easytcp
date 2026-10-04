//! This crate provides a small, generic TCP client and server for exchanging
//! typed messages, built on top of [`tokio`].
//!
//! # Overview
//!
//! - [`server::Server`] accepts connections, authenticates them against a
//!   pre-shared [`token::Token`], sends typed messages to individual clients or
//!   broadcasts to everyone, and forwards messages received from all clients
//!   through a single channel.
//! - [`client::Client`] connects to a [`server::Server`], authenticates with the
//!   same [`token::Token`], and exchanges typed messages with the server.
//!
//! Messages are serialized with [`postcard`] and framed with a 32-bit
//! big-endian length prefix. Client and server packet types are chosen by the
//! caller through the generic parameters of [`client::Client`] and
//! [`server::Server`].
#![warn(missing_docs)]

pub mod client;
pub mod server;
pub mod token;

mod error;
mod helpers;
mod traits;
