#![deny(warnings)]

extern crate futures;
extern crate tokio;

use tokio::io;
use tokio::net::TcpListener;
use tokio::prelude::*;

use std::collections::HashMap;
use std::env;
use std::io::BufReader;
use std::iter;
use std::sync::{Arc, Mutex};

fn main() {
    // Create the TCP listener we'll accept connections on.
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse().unwrap();

    let socket = TcpListener::bind(&addr).unwrap();
    println!("Listening on: {}", addr);

    let connections = Arc::new(Mutex::new(HashMap::new()));

    // The server task asynchronously iterates over and processes each incoming
    // connection.
    let srv = socket
        .incoming()
        .map_err(|e| println!("failed to accept socket; error = {:?}", e))
        .for_each(move |stream| {
            // The client's socket address
            let addr = stream.peer_addr().unwrap();

            println!("New Connection: {}", addr);

            // Split the TcpStream into two separate read and write handles.
            let (reader, writer) = stream.split();

            let (tx, rx) = futures::sync::mpsc::unbounded();
            connections.lock().unwrap().insert(addr, tx);

            let connections_inner = connections.clone();
            let reader = BufReader::new(reader);

            let socket_reader =
                stream::iter_ok::<_, io::Error>(iter::repeat(())).fold(reader, move |reader, _| {
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
                        .map(move |(reader, msg)| {
                            println!("{}: {:?}", addr, msg);
                            let mut conns = connections.lock().unwrap();

                            let iter = conns
                                .iter_mut()
                                .filter(|&(&k, _)| k != addr)
                                .map(|(_, v)| v);
                            for tx in iter {
                                if let Err(e) = tx.unbounded_send(format!("{}: {:?}\n", addr, msg)) {
                                    println!("Error: {}", e);
                                }
                            }

                            reader
                        })
                });

            let socket_writer = rx.fold(writer, |writer, msg| {
                io::write_all(writer, msg.into_bytes())
                    .map(|(writer, _)| writer)
                    .map_err(|_| ())
            });

            let connections = connections.clone();
            let connection = socket_reader
                .map_err(|_| ())
                .map(|_| ()).select(socket_writer.map(|_| ()));

            // Spawn a task to process the connection
            tokio::spawn(connection.then(move |_| {
                connections.lock().unwrap().remove(&addr);
                println!("Connection {} closed.", addr);
                Ok(())
            }));

            Ok(())
        });

    tokio::run(srv);
}
