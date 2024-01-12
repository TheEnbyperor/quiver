#![feature(extract_if)]

#[macro_use]
extern crate log;

mod connection;
mod qlog;

mod stream;

#[cfg(target_os = "linux")]
mod socket_linux;
#[cfg(not(target_os = "linux"))]
mod socket_other;

#[cfg(target_os = "linux")]
use socket_linux as socket;
#[cfg(not(target_os = "linux"))]
use socket_other as socket;

pub use connection::{Connection, NewConnections, ConnectionError, ConnectionRecv, QLogConfig};
pub use qlog::QLog;
pub use stream::{Stream, StreamID};
