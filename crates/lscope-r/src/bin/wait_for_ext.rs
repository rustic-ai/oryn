use lscope_r::server::RemoteServer;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let port = 9123;
    let server = RemoteServer::new(port);
    let handle = server.start().await.unwrap();
    println!("Server started on port {}", port);
    
    loop {
        let count = handle.command_tx.receiver_count();
        println!("Receiver count: {}", count);
        if count > 0 {
            println!("EXTENSION CONNECTED!");
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
