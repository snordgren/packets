use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{Shutdown, SocketAddr, TcpListener, ToSocketAddrs};
use std::time::{Duration};

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use socket2::{Domain, Protocol, Socket, Type};

use crate::*;

#[derive(Clone, Debug)]
pub struct ServerConfig {
  pub keep_alive_timeout: Option<Duration>,
}

impl Default for ServerConfig {
  fn default() -> Self {
    Self {
      keep_alive_timeout: None,
    }
  }
}

/// A simple server
///
/// # Examples
/// ## Echo server
/// ```
/// use packets::*;
/// use packets::server::*;
/// use std::net::SocketAddr;
/// use std::time::Instant;
///
/// // Bind the server to localhost at port 60000.
/// let mut server = Server::bind("localhost:60000", &ServerConfig::default()).unwrap();
/// let start_time = Instant::now();
/// loop {
///   server.accept_all(); // Accept all incoming connections.
///
///   // Using String as our packet type for this example, receive all incoming packets along with
///   // their addresses. Any Serde-compatible type can be used as the packet type.
///   let packets: Vec<(SocketAddr, String)> = server.receive_all().unwrap();
///   for (addr, packet) in packets {
///     server.send(addr, &packet); // Echo the data back to the client.
///   }
///
///   // For this example we shut the server down after 1 second.
///   if start_time.elapsed().as_secs() > 1 {
///     break;
///   }
/// }
///
/// server.shutdown(); // Close all connections to the server.
/// ```
#[derive(Debug)]
pub struct Server {
  connections: HashMap<SocketAddr, Connection>,
  pub listener: TcpListener,
}

impl Server {
  /// Attempt to accept a new connection.
  pub fn accept(&mut self) -> Result<Option<SocketAddr>> {
    self.listener.set_nonblocking(true)?;

    match self.listener.accept() {
      Ok((stream, addr)) => {
        let connection = Connection { stream, addr };
        self.connections.insert(addr, connection);
        Ok(Some(addr))
      }
      Err(err) => {
        if err.kind() == ErrorKind::WouldBlock {
          Ok(None)
        } else {
          Err(anyhow::Error::from(err))
        }
      }
    }
  }

  /// Use this function to accept all pending connections.
  pub fn accept_all(&mut self) -> Result<Vec<SocketAddr>> {
    let mut vec = Vec::new();
    loop {
      if let Some(accepted) = self.accept()? {
        vec.push(accepted);
      } else {
        break;
      }
    }
    Ok(vec)
  }

  /// Block until a new connection is accepted.
  pub fn accept_blocking(&mut self) -> Result<SocketAddr> {
    self.listener.set_nonblocking(false)?;
    let (stream, addr) = self.listener.accept()?;
    let connection = Connection { stream, addr };
    self.connections.insert(addr, connection);
    Ok(addr)
  }

  pub fn bind<B: ToSocketAddrs>(addr: B, config: &ServerConfig) -> Result<Server> {
    let socket_addr: SocketAddr = addr.to_socket_addrs()?.nth(0).unwrap();
    let domain = if socket_addr.is_ipv4() {
      Domain::ipv4()
    } else {
      Domain::ipv6()
    };

    let socket_type = Type::stream(); // This means TCP.

    let socket = Socket::new(domain, socket_type,
      Some(Protocol::tcp()))?;
    socket.set_keepalive(config.keep_alive_timeout)?;
    socket.bind(&socket_addr.into())?;
    socket.listen(128)?;

    let listener = socket.into_tcp_listener();
    Ok(Server { connections: HashMap::new(), listener })
  }

  pub fn block_until_receive_from(&mut self, addr: SocketAddr, timeout: Duration) -> Result<PacketReceiveStatus> {
    self.with_connection(addr, |conn| {
      crate::block_until_receive(&mut conn.stream, timeout)
    })
  }

  pub fn connections(&self) -> Vec<&Connection> {
    self.connections.values().collect()
  }

  /// Iterate over connections until we find one that is
  pub fn receive<A: Serialize + DeserializeOwned>(&mut self) -> Result<Option<(SocketAddr, A)>> {
    for conn in self.connections.values_mut() {
      let result: Option<A> = read_packet::<A>(&mut conn.stream, false)?;
      if result.is_some() {
        return Ok(Some((conn.addr, result.unwrap())))
      }
    }

    Ok(None)
  }

  pub fn receive_all<A: Serialize + DeserializeOwned>(&mut self) -> Result<Vec<(SocketAddr, A)>> {
    let mut vec = Vec::new();
    loop {
      if let Some(packet) = self.receive()? {
        vec.push(packet);
      } else {
        break;
      }
    }
    Ok(vec)
  }

  pub fn receive_from<A: Serialize + DeserializeOwned>(&mut self, addr: SocketAddr) -> Result<Option<A>> {
    self.with_connection(addr, |conn| {
      crate::read_packet::<A>(&mut conn.stream, false)
    })
  }

  /// Send a packet to a specific connection identified by its address.
  pub fn send<A: Serialize + DeserializeOwned>(&mut self, addr: SocketAddr, packet: &A) -> Result<()> {
    self.with_connection(addr, |conn| {
      write_packet(&mut conn.stream, packet)?;
      Ok(())
    })
  }

  /// Send a packet to all connections.
  pub fn send_global<A: Serialize + DeserializeOwned>(&mut self, packet: &A) -> Result<()> {
    for conn in self.connections.values_mut() {
      write_packet(&mut conn.stream, packet)?;
    }

    Ok(())
  }

  /// Shuts this server down, closing all connections, then drops this object. The server cannot
  /// be used after it has been shut down as the Server object is moved into this function and then
  /// dropped.
  pub fn shutdown(mut self) -> Result<()> {
    for conn in &mut self.connections.values_mut() {
      conn.stream.shutdown(Shutdown::Both)?;
    }
    Ok(())
  }

  /// Run a closure on a
  pub fn with_connection<A, F>(&mut self, addr: SocketAddr, f: F) -> Result<A> where
    F: FnOnce(&mut Connection) -> Result<A> {

    if let Some(conn) = self.connections.get_mut(&addr) {
      match f(conn) {
        Ok(res) => Ok(res),
        Err(err) => {
          // We had an error sending to this client, so we remove their connection.
          if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            let kind = io_err.kind();

            // TODO Add more error types here that should remove the connection from our list.
            if kind == ErrorKind::ConnectionAborted {
              self.connections.remove(&addr);
            }
          }

          Err(err)
        }
      }
    } else {
      let kind = ErrorKind::NotConnected;
      Err(anyhow::Error::from(std::io::Error::new(kind, "Connection not found.")))
    }
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::*;
  use crate::client::*;

  #[test]
  pub fn test_server_send_packet() -> Result<()> {
    let addr = "localhost:60003";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;
    let client_addr = server.accept_blocking()?;
    let packet = 42;
    server.send_global(&packet);
    client.block_until_receive(Duration::from_millis(2000));
    let received_packet = client.receive()?
      .expect("Unable to read packet.");
    assert_eq!(packet, received_packet);

    Ok(())
  }

  #[test]
  pub fn test_server_accept_non_blocking() -> Result<()> {
    let addr = "localhost:60003";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let client_addr = server.accept()?;
    assert_eq!(None, client_addr);

    Ok(())
  }

  #[test]
  pub fn test_server_block_until_receive_from_timeout() -> Result<()> {

    let addr = "localhost:60002";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;

    let client_addr = server.accept_blocking()?;

    let start_time = Instant::now();
    let millis = 200;
    let status = server.block_until_receive_from(client_addr, Duration::from_millis(millis))?;
    let elapsed = start_time.elapsed().as_millis();
    let diff = ((elapsed as i64) - (millis as i64)).abs();

    assert_eq!(PacketReceiveStatus::TimedOut, status);
    assert!(diff < 5);

    Ok(())
  }

  #[test]
  pub fn test_server_connection_lost() -> Result<()> {

    let addr = "localhost:60007";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;
    let client_addr = server.accept_blocking()?;
    assert_eq!(1, server.connections().len());
    client.shutdown();

    // Give the client a second to shut down.
    std::thread::sleep(Duration::from_secs(1));

    // Attempt to send to the client, causing it to be removed when the connection doesn't work out.
    server.send(client_addr, &42)
      .expect_err("Expected Err here.");

    assert_eq!(0, server.connections().len());

    Ok(())
  }

  #[test]
  pub fn test_server_non_blocking_receive_fail() -> Result<()> {
    let addr = "localhost:60006";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;

    // Attempt to receive when there is nothing to receive.
    let res: Option<(SocketAddr, Vec<u8>)> = server.receive()?;
    assert_eq!(None, res);

    Ok(())
  }

  #[test]
  pub fn test_server_shutdown() -> Result<()> {
    let addr = "localhost:60004";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;
    let client_addr = server.accept_blocking()?;
    assert_eq!(1, server.connections().len());
    server.shutdown();

    // Give the server a second to shut down.
    std::thread::sleep(Duration::from_secs(1));

    // Attempt to send to the client, causing it to be removed when the connection doesn't work out.
    client.send(&42)
      .expect_err("Expected Err here.");
    assert!(!client.is_connected());

    Ok(())
  }
}