# packets

Packets is a small library that makes writing packet-based TCP
servers and clients a bit easier.

## Creating a Server
    use packets::*;
    use packets::server::*;
    use std::net::SocketAddr;
    
    // Bind the server to localhost at port 60000.
    let mut server = Server::bind("localhost:60000", &ServerConfig::default()).unwrap();
    
    // Accept all incoming connections.
    server.accept_all(); 
    
    // Receive all incoming packets, using String as our packet type.
    let packets: Vec<(SocketAddr, String)> = server.receive_all().unwrap();
    for (addr, packet) in packets {
      server.send(addr, &packet); // Echo the data back to the client.
    }
    
     // Close all connections to the server and consume the Server object.
    server.shutdown();

## Creating a Client
    use packets::*;
    use packets::client::*;
    
    let mut client = Client::connect("localhost:60000", &ClientConfig::default());
    
    // Receive all incoming packets.
    // In this example we use String as our Packet type.
    let packets: Vec<String> = client.receive_all();

    // Send a packet to the server, using String as our packet type for this example.
    client.send("Hello, world!".to_string());

    // Shut this client down, consuming it.    
    client.shutdown();
