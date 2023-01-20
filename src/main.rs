use bytes::{BufMut, BytesMut};

use futures::future::join_all;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
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

    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());

    let token = CancellationToken::new();
    let token_clone = token.clone();

    let server = start_server(pid, addr, token_clone).await?;

    wait_for_signal(pid, token).await?;

    server.await?;

    Ok(())
}

async fn wait_for_signal(pid: u32, token: CancellationToken) -> Result<(), Box<dyn Error>> {
    let mut signal_stream = signal(SignalKind::hangup())?;
    signal_stream.recv().await.ok_or("can't catch the signal")?;
    println!("Process {pid} received hangup signal");
    token.cancel();
    Ok(())
}

async fn start_server(
    pid: u32,
    addr: String,
    token: CancellationToken,
) -> Result<JoinHandle<()>, Box<dyn Error>> {
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.set_reuseport(true)?;
    socket.bind(addr.parse()?)?;
    let listener = socket.listen(MAX_LISTENERS)?;
    println!("Process {pid} listening on: {}", addr);

    let server = tokio::spawn(async move {
        handle_connection(pid, listener, token)
            .await
            .expect("failed to handle connection");
    });

    Ok(server)
}

#[derive(Default)]
struct Tasks<T> {
    tasks: Vec<JoinHandle<T>>,
}

impl<T> Tasks<T> {
    fn add(&mut self, task: JoinHandle<T>) {
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

type SharedTasks<T> = Arc<Mutex<Tasks<T>>>;

async fn handle_connection(
    pid: u32,
    listener: TcpListener,
    token: CancellationToken,
) -> Result<(), Box<dyn Error>> {
    let tasks = SharedTasks::default();

    let token_clone = token.clone();
    let tasks_clone = Arc::clone(&tasks);
    let remove_finished_tasks_worker = tokio::spawn(async move {
        task_cleanup_worker(token_clone, tasks_clone).await;
    });

    let postfix = Arc::new(format!(": from {pid}\n"));

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                // do not accept new connections
                drop(listener);

                // wait until all other connections are closed
                let _ = remove_finished_tasks_worker.await;

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
                    protocol(&mut socket, postfix_clone).await.expect("failed to handle protocol");
                }));
            }

            else => {
                continue;
            }
        }
    }

    Ok(())
}

async fn protocol(stream: &mut TcpStream, postfix: Arc<String>) -> Result<(), Box<dyn Error>> {
    let mut buf = BytesMut::with_capacity(BUFFER_SIZE);

    loop {
        let n = stream
            .read_buf(&mut buf)
            .await?;

        if n == 0 {
            break;
        }

        buf.put(postfix.as_bytes());

        stream
            .write_buf(&mut buf)
            .await?;
    }

    Ok(())
}

async fn task_cleanup_worker<T>(token: CancellationToken, tasks: SharedTasks<T>) {
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                break;
            }

            _ = sleep(Duration::from_secs(CLEAN_INTERVAL)) => {
                let mut tasks = tasks.lock().await;
                tasks.remove_finished_tasks();
            }
        }
    }
}
