use std::io::ErrorKind;
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use socket2::{Domain, Protocol, Socket, Type};

use crate::*;

#[derive(Debug)]
pub struct ClientConfig {
  pub connect_timeout: Option<Duration>,
  pub keep_alive_timeout: Option<Duration>
}

impl Default for ClientConfig {
  fn default() -> Self {
    Self {
      connect_timeout: None,
      keep_alive_timeout: None,
    }
  }
}

#[derive(Debug)]
pub struct Client {
  is_connected: bool,
  pub stream: TcpStream,
}

impl Client {
  pub fn block_until_receive(&mut self, timeout: Duration) -> Result<PacketReceiveStatus> {
    crate::block_until_receive(&mut self.stream, timeout)
  }

  pub fn connect<B: ToSocketAddrs>(addr: B, config: &ClientConfig) -> Result<Client> {
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
    if let Some(connect_timeout) = config.connect_timeout {
      socket.connect_timeout(&socket_addr.into(), connect_timeout)?;
    } else {
      socket.connect(&socket_addr.into())?;
    }

    let stream = socket.into_tcp_stream();
    Ok(Client { is_connected: true, stream })
  }

  pub fn is_connected(&self) -> bool {
    self.is_connected
  }

  pub fn receive<A: Serialize + DeserializeOwned>(&mut self) -> Result<Option<A>> {
    read_packet::<A>(&mut self.stream, false)
  }

  /// Block the thread until a packet has been received from the thread.
  pub fn receive_blocking<A: Serialize + DeserializeOwned>(&mut self, timeout: Duration) -> Result<Option<A>> {
    let res = self.block_until_receive(timeout)?;
    if res == PacketReceiveStatus::TimedOut {
      return Ok(None)
    }

    read_packet::<A>(&mut self.stream, true)
  }

  pub fn send<A: Serialize + DeserializeOwned>(&mut self, packet: &A) -> Result<()> {
    match write_packet(&mut self.stream, packet) {
      Ok(_) => Ok(()),
      Err(err) => {
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
          let kind = io_err.kind();
          if kind == ErrorKind::ConnectionAborted {
            self.is_connected = false;
          }
        }

        Err(err)
      }
    }
  }

  /// Note: shutting down the Client is not immediate.
  pub fn shutdown(self) -> Result<()> {
    self.stream.set_nonblocking(false)?;
    self.stream.shutdown(Shutdown::Both)?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use std::{io, thread};
  use std::time::Duration;

  use crate::PacketReceiveStatus::Received;
  use crate::server::*;

  use super::*;

  #[test]
  pub fn test_client_send_packet() -> Result<()> {
    let addr = "localhost:60001";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;
    let client_addr = server.accept_blocking()?;
    let packet = 42;
    client.send(&packet);
    server.block_until_receive_from(client_addr, Duration::from_millis(2000));
    let (sender_addr, received_packet) = server.receive()?
      .expect("Unable to read packet.");
    assert_eq!(packet, received_packet);
    assert_eq!(client_addr, sender_addr);

    Ok(())
  }

  #[test]
  pub fn test_client_block_until_receive_timeout() -> Result<()> {
    let addr = "localhost:60005";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;
    server.accept_blocking()?;

    let millis = 200;
    let start_time = Instant::now();
    let result = client.block_until_receive(Duration::from_millis(millis))?;
    assert_eq!(PacketReceiveStatus::TimedOut, result);
    let duration = start_time.elapsed().as_millis();
    let difference = ((duration as i64) - (millis as i64)).abs();
    assert!(difference < 5); // Difference should be less than 5 ms.

    Ok(())
  }

  /// Tests that the
  #[test]
  pub fn test_client_connect_timeout() -> Result<()> {
    let millis = 1000;
    let start_time = Instant::now();
    Client::connect("localhost:60006", &ClientConfig {
      connect_timeout: Some(Duration::from_millis(millis)),
      ..ClientConfig::default()
    })
      .expect_err("Expected the client to fail to connect.");
    let difference = ((start_time.elapsed().as_millis() as i64) - (millis as i64)).abs();
    assert!(difference < 10); // 10 milliseconds of tolerance on the connect timeout.
    Ok(())
  }

  #[test]
  pub fn test_client_non_blocking_receive_timeout() -> Result<()> {
    let addr = "localhost:60008";
    let mut server = Server::bind(addr, &ServerConfig::default())?;
    let mut client = Client::connect(addr, &ClientConfig::default())?;
    server.accept_blocking()?;

    let millis = 200;
    let start_time = Instant::now();
    let result: Option<String> = client.receive_blocking(Duration::from_millis(millis))?;
    assert_eq!(None, result);
    let duration = start_time.elapsed().as_millis();
    let difference = ((duration as i64) - (millis as i64)).abs();
    assert!(difference < 5); // Difference should be less than 5 ms.

    Ok(())
  }
}