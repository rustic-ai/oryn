use futures::{SinkExt, StreamExt};
use oryn_engine::protocol::{Action, ScannerAction, ScannerProtocolResponse};
use oryn_r::server::RemoteServer;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

async fn connect_simulated_client(port: u16) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
    let url = format!("ws://localhost:{}", port);
    // Retry connection logic
    for _ in 0..10 {
        if let Ok((ws_stream, _)) = connect_async(&url).await {
            return ws_stream;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("Failed to connect to simulated server");
}

#[tokio::test]
async fn test_server_connection_and_messaging() {
    // 1. Start Server on random port (or fixed for test uniqueness)
    let port = 9050;
    let server = RemoteServer::new(port);
    let handle = server.start().await.expect("Failed to start server");

    // 2. Connect simulates extension
    let mut client_ws = connect_simulated_client(port).await;

    // 3. Send command from Server -> Client
    let test_cmd = ScannerAction::Scan(oryn_engine::protocol::ScanRequest {
        max_elements: None,
        monitor_changes: false,
        include_hidden: false,
        view_all: false,
        near: None,
        viewport_only: false,
    });

    // Wrap in Action
    let action = Action::Scanner(test_cmd);

    // Send via server handle
    handle
        .command_tx
        .send(action.clone())
        .expect("Failed to send command");

    // 4. Verify Client receives it
    if let Some(msg) = client_ws.next().await {
        let msg = msg.expect("WS error");
        let text = msg.to_string();
        let received: Action = serde_json::from_str(&text).expect("Failed to deserialize");

        // Check variant matches
        if let Action::Scanner(ScannerAction::Scan(_)) = received {
            // OK
        } else {
            panic!("Wrong command received: {:?}", received);
        }
    } else {
        panic!("Client stream ended unexpectedly");
    }

    // 5. Send response from Client -> Server
    let test_resp = ScannerProtocolResponse::Ok {
        data: Box::new(oryn_engine::protocol::ScannerData::Value(
            serde_json::json!({ "foo": "bar" }),
        )),
        warnings: vec![],
    };

    let resp_str = serde_json::to_string(&test_resp).unwrap();
    client_ws
        .send(tokio_tungstenite::tungstenite::Message::Text(resp_str))
        .await
        .unwrap();

    // 6. Verify Server receives it via handle
    // We need to lock the receiver
    let mut rx = handle.response_rx.lock().await;
    // recv might take a moment
    let received_resp = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("Timeout waiting for response")
        .expect("Channel closed");

    if let ScannerProtocolResponse::Ok { .. } = received_resp {
        // OK
    } else {
        panic!("Server received error response: {:?}", received_resp);
    }
}
