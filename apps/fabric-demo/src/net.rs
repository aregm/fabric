//! Shared networking rules: every service listens on loopback only (section 5.7).

use std::io;
use std::net::SocketAddr;

use tokio::net::TcpListener;

/// Binds a loopback address; any other address is refused. Port 0 picks a free port.
pub async fn bind_loopback(address: SocketAddr) -> io::Result<TcpListener> {
    if !address.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("demo services listen on loopback only, not {address}"),
        ));
    }
    TcpListener::bind(address).await
}

/// Checks a configured listen address before anything starts.
pub fn check_loopback(address: SocketAddr) -> Result<(), String> {
    if address.ip().is_loopback() {
        Ok(())
    } else {
        Err(format!(
            "demo services listen on loopback only, not {address}"
        ))
    }
}
