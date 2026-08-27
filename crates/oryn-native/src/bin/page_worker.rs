use std::io::{BufReader, stdin, stdout};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    oryn_native::worker::serve(BufReader::new(stdin().lock()), stdout().lock()).await
}
