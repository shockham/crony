#![deny(warnings)]

extern crate tokio;
extern crate futures;

use tokio::io;
use tokio::net::TcpListener;
use tokio::prelude::*;

use std::collections::HashMap;
use std::iter;
use std::env;
use std::io::{BufReader};
use std::sync::{Arc, Mutex};

fn main() {
    // Create the TCP listener we'll accept connections on.
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse().unwrap();

    let socket = TcpListener::bind(&addr).unwrap();
    println!("Listening on: {}", addr);

    // This is running on the Tokio runtime, so it will be multi-threaded. The
    // `Arc<Mutex<...>>` allows state to be shared across the threads.
    let connections = Arc::new(Mutex::new(HashMap::new()));

    // The server task asynchronously iterates over and processes each incoming
    // connection.
    let srv = socket.incoming()
        .map_err(|e| println!("failed to accept socket; error = {:?}", e))
        .for_each(move |stream| {
            // The client's socket address
            let addr = stream.peer_addr().unwrap();

            println!("New Connection: {}", addr);

            // Split the TcpStream into two separate read and write handles.
            let (reader, writer) = stream.split();

            // Create a channel for our stream, which other sockets will use to
            // send us messages. Then register our address with the stream to send
            // data to us.
            let (tx, rx) = futures::sync::mpsc::unbounded();
            connections.lock().unwrap().insert(addr, tx);

            // Define here what we do for the actual I/O. That is, read a bunch of
            // lines from the socket and dispatch them while we also write any lines
            // from other sockets.
            let connections_inner = connections.clone();
            let reader = BufReader::new(reader);

            // Model the read portion of this socket by mapping an infinite
            // iterator to each line off the socket. This "loop" is then
            // terminated with an error once we hit EOF on the socket.
            let iter = stream::iter_ok::<_, io::Error>(iter::repeat(()));

            let socket_reader = iter.fold(reader, move |reader, _| {
                // Move the connection state into the closure below.
                let connections = connections_inner.clone();
                // Read a line off the socket, failing if we're at EOF
                io::read_until(reader, b'\n', Vec::new())
                    .and_then(|(reader, vec)| {
                        if vec.len() == 0 {
                            Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"))
                        } else {
                            Ok((reader, vec))
                        }
                    })
                    .map(move |(reader, vec)| {
                        let message = String::from_utf8(vec);
                        println!("{}: {:?}", addr, message);
                        let mut conns = connections.lock().unwrap();

                        if let Ok(msg) = message {
                            // For each open connection except the sender, send the
                            // string via the channel.
                            let iter = conns.iter_mut()
                                            .filter(|&(&k, _)| k != addr)
                                            .map(|(_, v)| v);
                            for tx in iter {
                                tx.unbounded_send(format!("{}: {}", addr, msg)).unwrap();
                            }
                        } else {
                            let tx = conns.get_mut(&addr).unwrap();
                            tx.unbounded_send("You didn't send valid UTF-8.".to_string()).unwrap();
                        }

                        reader
                    })
            });

            // Whenever we receive a string on the Receiver, we write it to
            // `WriteHalf<TcpStream>`.
            let socket_writer = rx.fold(writer, |writer, msg| {
                let amt = io::write_all(writer, msg.into_bytes());
                let amt = amt.map(|(writer, _)| writer);
                amt.map_err(|_| ())
            });

            // Now that we've got futures representing each half of the socket, we
            // use the `select` combinator to wait for either half to be done to
            // tear down the other. Then we spawn off the result.
            let connections = connections.clone();
            let socket_reader = socket_reader.map_err(|_| ());
            let connection = socket_reader.map(|_| ()).select(socket_writer.map(|_| ()));

            // Spawn a task to process the connection
            tokio::spawn(connection.then(move |_| {
                connections.lock().unwrap().remove(&addr);
                println!("Connection {} closed.", addr);
                Ok(())
            }));

            Ok(())
        });

    // execute server
    tokio::run(srv);
}
