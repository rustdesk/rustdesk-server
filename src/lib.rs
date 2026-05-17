pub mod codec {
    pub use core_common::rendezvous_codec::*;
}
mod rendezvous_server;
pub use rendezvous_server::*;
pub mod api;
pub mod common;
pub mod database;
pub mod database_simple;
pub mod device_api;
pub mod password_reset;
pub mod peer;
mod version;
pub mod views;
pub mod subscription;
pub mod web;
