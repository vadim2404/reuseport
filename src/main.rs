use bytes::{BufMut, BytesMut};

use futures::future::join_all;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

use std::env;
use std::error::Error;
use std::process;
use std::sync::Arc;

const MAX_LISTENERS: u32 = 1024;
const BUFFER_SIZE: usize = 1024;
const CLEAN_INTERVAL: u64 = 5;

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
    let listener = socket.listen(MAX_LISTENERS)?;
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

#[derive(Default)]
struct Tasks {
    tasks: Vec<JoinHandle<()>>,
}

impl Tasks {
    fn add(&mut self, task: JoinHandle<()>) {
        self.tasks.push(task);
    }

    async fn join_all(&mut self) {
        join_all(self.tasks.drain(..)).await;
    }

    fn remove_finished_tasks(&mut self) {
        self.tasks.retain(|task| !task.is_finished());
    }

    fn len(&self) -> usize {
        self.tasks.len()
    }
}

async fn handle_connection(
    pid: u32,
    listener: TcpListener,
    postfix: String,
    cancellation_token: CancellationToken,
) -> Result<(), Box<dyn Error>> {
    let tasks = Arc::new(Mutex::new(Tasks::default()));

    let cancellation_token_clone = cancellation_token.clone();
    let tasks_clone = Arc::clone(&tasks);
    let remove_finish_tasks = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancellation_token_clone.cancelled() => {
                    break;
                }

                _ = sleep(Duration::from_secs(CLEAN_INTERVAL)) => {
                    let mut tasks = tasks_clone.lock().await;
                    tasks.remove_finished_tasks();
                }
            }
        }
    });

    let postfix = Arc::new(postfix);

    loop {
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                // do not accept new connections
                drop(listener);

                // wait until all other connections are closed
                let _ = remove_finish_tasks.await;

                let mut tasks = tasks.lock().await;
                println!("process {pid} closing tasks: {}", tasks.len());
                tasks.join_all().await;
                break;
            }

            Ok((mut socket, addr)) = listener.accept() => {
                println!("Process {pid} accepted connection from: {addr}");

                let postfix_clone = Arc::clone(&postfix);

                let mut tasks = tasks.lock().await;
                tasks.add(tokio::spawn(async move {
                    let mut buf = BytesMut::with_capacity(BUFFER_SIZE);

                    loop {
                        let n = socket
                            .read_buf(&mut buf)
                            .await
                            .expect("failed to read data from socket");

                        if n == 0 {
                            break;
                        }

                        buf.put(postfix_clone.as_bytes());

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
