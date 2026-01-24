use futures::{SinkExt, StreamExt};
use oryn_engine::protocol::{ScannerProtocolResponse, ScannerRequest};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info};

#[derive(Clone)]
pub struct RemoteServer {
    port: u16,
    // Channel to send commands to the active connection loop
    command_tx: broadcast::Sender<ScannerRequest>,
    // Receiver for responses (held by the Backend, but we might need to store it to clone/init?)
    // Actually, we'll expose a method to get a receiver or return responses.
    // Simpler: Shared state for the latest response? Or a response channel that the Backend subscribes to?
    // Let's use a broadcast channel for commands (1 sender -> N connections (usually 1))
    // And an mpsc for responses (N connections -> 1 receiver)
    // The Backend will hold the response_rx.
}

pub struct ServerHandle {
    pub command_tx: broadcast::Sender<ScannerRequest>,
    pub response_rx: Arc<Mutex<mpsc::Receiver<ScannerProtocolResponse>>>,
}

impl RemoteServer {
    pub fn new(port: u16) -> Self {
        let (command_tx, _) = broadcast::channel(100);
        Self { port, command_tx }
    }

    pub async fn start(&self) -> Result<ServerHandle, Box<dyn std::error::Error>> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(&addr).await?;
        info!("Remote Server listening on: {}", addr);
        println!("INFO: Remote Server listening on: {}", addr);

        let (response_tx, response_rx) = mpsc::channel(100);
        let command_tx = self.command_tx.clone();

        let server_cmd_tx = command_tx.clone();

        tokio::spawn(async move {
            info!("Server accept loop started");
            while let Ok((stream, _)) = listener.accept().await {
                let peer = stream
                    .peer_addr()
                    .expect("connected streams should have a peer address");
                info!("Accepted TCP connection from: {}", peer);
                println!("INFO: Accepted TCP connection from: {}", peer);

                let cmd_rx = server_cmd_tx.subscribe();
                let resp_tx = response_tx.clone();

                tokio::spawn(accept_connection(stream, cmd_rx, resp_tx));
            }
        });

        Ok(ServerHandle {
            command_tx,
            response_rx: Arc::new(Mutex::new(response_rx)),
        })
    }
}

async fn accept_connection(
    stream: TcpStream,
    mut cmd_rx: broadcast::Receiver<ScannerRequest>,
    resp_tx: mpsc::Sender<ScannerProtocolResponse>,
) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => {
            println!("INFO: WebSocket Handshake Successful");
            ws
        }
        Err(e) => {
            error!("Error during the websocket handshake occurred: {}", e);
            return;
        }
    };

    info!("New WebSocket connection: established");
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Loop handling both incoming commands (to send to WS) and incoming WS messages (responses)
    loop {
        tokio::select! {
            // Receive command from Backend -> Send to Extension
            Ok(cmd) = cmd_rx.recv() => {
                let json = serde_json::to_string(&cmd).unwrap(); // Handle error properly in production
                if let Err(e) = ws_sender.send(Message::Text(json)).await {
                    error!("Failed to send message to WS: {}", e);
                    break;
                }
            }

            // Receive message from Extension -> Send to Backend
            Some(msg) = ws_receiver.next() => {
                match msg {
                    Ok(Message::Text(text)) => {
                        // Parse as ScannerProtocolResponse
                        match serde_json::from_str::<ScannerProtocolResponse>(&text) {
                            Ok(resp) => {
                                if let Err(e) = resp_tx.send(resp).await {
                                     error!("Failed to forward response to backend: {}", e);
                                     break;
                                }
                            },
                            Err(e) => {
                                error!("Failed to parse response from extension: {} | Text: {}", e, text);
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket closed");
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}
