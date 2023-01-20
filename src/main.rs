use bytes::{BufMut, BytesMut};

use futures::future::join_all;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket};
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::sync::CancellationToken;

use std::env;
use std::error::Error;
use std::process;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let pid = process::id();

    println!("Process {pid} running");

    let postfix = format!(": from {pid}\n");

    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());

    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.set_reuseport(true)?;
    socket.bind(addr.parse()?)?;
    let listener = socket.listen(1024)?;
    println!("Process {pid} listening on: {}", addr);

    let mut signal_stream = signal(SignalKind::hangup())?;
    let token = CancellationToken::new();
    let token_clone = token.clone();

    let jh = tokio::spawn(async move {
        handle_connection(pid, listener, postfix, token_clone)
            .await
            .expect("failed to handle connection");
    });

    signal_stream
        .recv()
        .await
        .ok_or("error in handling signal")?;
    println!("Process {pid} received hangup signal");
    token.cancel();

    jh.await?;

    Ok(())
}

async fn handle_connection(
    pid: u32,
    listener: TcpListener,
    postfix: String,
    cancellation_token: CancellationToken,
) -> Result<(), Box<dyn Error>> {
    let mut tasks = vec![];
    let postfix = Arc::new(postfix);

    loop {
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                // do not accept new connections
                drop(listener);

                // wait until all other connections are closed
                println!("process {pid} closing tasks: {}", tasks.len());
                join_all(tasks).await;
                break;
            }

            Ok((mut socket, addr)) = listener.accept() => {
                println!("Process {pid} accepted connection from: {addr}");

                let postfix_clone = postfix.clone();

                tasks.push(tokio::spawn(async move {
                    let mut buf = BytesMut::with_capacity(1024);

                    loop {
                        let n = socket
                            .read_buf(&mut buf)
                            .await
                            .expect("failed to read data from socket");

                        if n == 0 {
                            break;
                        }

                        buf.put(&postfix_clone.as_bytes()[..]);

                        socket
                            .write_buf(&mut buf)
                            .await
                            .expect("failed to write data to socket");
                    }
                }));
            }

            else => {
                continue;
            }
        }
    }

    Ok(())
}
