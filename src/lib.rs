use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use anyhow::Result;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod client;
pub mod server;

#[derive(Copy, Clone, Debug, Eq, Ord, PartialOrd, PartialEq)]
pub enum PacketReceiveStatus {
  Received,
  TimedOut
}

#[derive(Debug)]
pub struct Connection {
  pub addr: SocketAddr,
  pub stream: TcpStream,
}

pub(crate) fn block_until_receive(stream: &mut TcpStream, timeout: Duration) -> Result<PacketReceiveStatus> {
  // Make sure we have a non-blocking TcpStream. We can't use a blocking TcpStream as it does not
  // support timeouts. So we have to poll the stream.
  stream.set_nonblocking(true)?;

  let start_time = Instant::now();

  // The size of the buffer has to be more than 4 bytes, otherwise we can't peek and see if more
  // than 4 bytes are in the buffer.
  // If there are 4 or fewer bytes in the buffer we don't want to read the packet yet because only
  // the size descriptor has been received. We want at least 1 byte of the packet to have been
  // received before we retrieve it.
  let mut buf = [0u8; 5];

  loop {
    if start_time.elapsed() > timeout {
      break;
    }

    match stream.peek(&mut buf) {
      Ok(peeked) => {
        if peeked > 4 {
          return Ok(PacketReceiveStatus::Received);
        } else {
          continue;
        }
      }
      Err(err) => {
        if err.kind() == ErrorKind::WouldBlock {
          continue;
        } else {
          return Err(anyhow::Error::from(err));
        }
      }
    }
  }

  Ok(PacketReceiveStatus::TimedOut)
}

pub(crate) fn read_packet<A: Serialize + DeserializeOwned>(stream: &mut TcpStream, blocking: bool) -> Result<Option<A>> {
  let mut buf = [0u8; 5];
  stream.set_nonblocking(!blocking)?;

  let peek_bytes_res = stream.peek(&mut buf);
  let peek_bytes = match peek_bytes_res {
    Ok(peek_bytes) => peek_bytes,
    Err(err) => {
      return if err.kind() == ErrorKind::WouldBlock {
        // We can't peek 8 bytes
        Ok(None)
      } else {
        Result::Err(anyhow::Error::from(err))
      }
    }
  };

  let mut result = Ok(None);

  // The size marker is 4 bytes, if we have more than the size marker then we want to read the
  // entire packet.
  if peek_bytes > 4 {

    // We set nonblocking to false so that we can block until the entire packet has been read.
    stream.set_nonblocking(false)?;

    let bytes = stream.read_u32::<LittleEndian>()? as usize;

    // Initialize a vector with the exact right size for us to read from the packet.
    let mut packet_bytes = vec![0; bytes];

    match stream.read_exact(&mut packet_bytes) {
      Ok(_) => {}
      Err(err) => {
        let kind = err.kind();
        return if kind == ErrorKind::WouldBlock {
          Ok(None)
        } else {
          Err(anyhow::Error::from(err))
        }
      }
    }

    let packet = bincode::deserialize(&packet_bytes)?;
    result = Ok(Some(packet))
  }

  result
}

pub(crate) fn write_packet<A: Serialize + DeserializeOwned>(stream: &mut TcpStream, packet: &A) -> Result<()> {
  let bytes = bincode::serialize(packet)?;
  stream.set_nonblocking(false)?;
  stream.write_u32::<LittleEndian>(bytes.len() as u32)?;
  stream.write_all(&bytes)?;
  stream.set_nonblocking(true)?;
  Ok(())
}
