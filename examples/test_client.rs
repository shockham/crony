#![deny(warnings)]

extern crate bytes;
extern crate futures;
extern crate tokio;
extern crate tokio_codec;
extern crate tokio_io;

use std::env;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::thread;

use futures::sync::mpsc;
use tokio::prelude::*;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();

    // Parse what address we're going to connect to
    let addr = args
        .first()
        .unwrap_or_else(|| panic!("this program requires at least one argument"));
    let addr = addr.parse::<SocketAddr>().unwrap();

    let (stdin_tx, stdin_rx) = mpsc::channel(0);
    thread::spawn(|| read_stdin(stdin_tx));
    let stdin_rx = stdin_rx.map_err(|_| panic!()); // errors not possible on rx

    let stdout = tcp::connect(&addr, Box::new(stdin_rx));

    let mut out = io::stdout();

    tokio::run({
        stdout
            .for_each(move |chunk| {
                out.write_all(&chunk)
            })
            .map_err(|e| println!("error reading stdout; error = {:?}", e))
    });
}

mod codec {
    use bytes::{BufMut, BytesMut};
    use std::io;
    use tokio_codec::{Decoder, Encoder};

    pub struct Bytes;

    impl Decoder for Bytes {
        type Item = BytesMut;
        type Error = io::Error;

        fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<BytesMut>> {
            if buf.len() > 0 {
                let len = buf.len();
                Ok(Some(buf.split_to(len)))
            } else {
                Ok(None)
            }
        }
    }

    impl Encoder for Bytes {
        type Item = Vec<u8>;
        type Error = io::Error;

        fn encode(&mut self, data: Vec<u8>, buf: &mut BytesMut) -> io::Result<()> {
            buf.put(&data[..]);
            Ok(())
        }
    }
}

mod tcp {
    use tokio;
    use tokio::net::TcpStream;
    use tokio::prelude::*;
    use tokio_codec::Decoder;

    use bytes::BytesMut;
    use codec::Bytes;

    use std::io;
    use std::net::SocketAddr;

    pub fn connect(
        addr: &SocketAddr,
        stdin: Box<Stream<Item = Vec<u8>, Error = io::Error> + Send>,
    ) -> Box<Stream<Item = BytesMut, Error = io::Error> + Send> {
        let tcp = TcpStream::connect(addr);

        Box::new(
            tcp.map(move |stream| {
                let (sink, stream) = Bytes.framed(stream).split();

                tokio::spawn(stdin.forward(sink).then(|result| {
                    if let Err(e) = result {
                        panic!("failed to write to socket: {}", e)
                    }
                    Ok(())
                }));

                stream
            }).flatten_stream(),
        )
    }
}

// Our helper method which will read data from stdin and send it along the
// sender provided.
fn read_stdin(mut tx: mpsc::Sender<Vec<u8>>) {
    let mut stdin = io::stdin();
    loop {
        let mut buf = vec![0; 1024];
        let n = match stdin.read(&mut buf) {
            Err(_) | Ok(0) => break,
            Ok(n) => n,
        };
        buf.truncate(n);
        tx = match tx.send(buf).wait() {
            Ok(tx) => tx,
            Err(_) => break,
        };
    }
}
