use std::{
    io::{Read, Write},
    net::TcpListener,
};

use oryn_native::{Browser, ContextOptions};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?;
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        for request_index in 0..2 {
            let (mut stream, _) = listener.accept()?;
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = stream.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            let cookie_present = request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("cookie: oryn_session=parent-owned"));
            let (body, cookie) = if request_index == 0 {
                ("<!doctype html><title>cookie-set</title>", true)
            } else if cookie_present {
                ("<!doctype html><title>cookie-present</title>", false)
            } else {
                ("<!doctype html><title>cookie-missing</title>", false)
            };
            let cookie_header = if cookie {
                "Set-Cookie: oryn_session=parent-owned; Path=/; HttpOnly\r\n"
            } else {
                ""
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{cookie_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
            stream.flush()?;
        }
        Ok(())
    });

    let browser = Browser::new();
    let context = browser.new_context_with_options(ContextOptions::loopback_test());
    let first = context.new_page();
    let second = context.new_page();
    first.goto(format!("http://{address}/set")).await?;
    second.goto(format!("http://{address}/check")).await?;
    let title = second.evaluate("document.title").await?;
    if title != "cookie-present" {
        return Err(
            format!("parent cookie jar was not shared across page workers: {title}").into(),
        );
    }
    first.close().await?;
    second.close().await?;
    server.join().map_err(|_| "cookie server panicked")??;
    println!("parent-owned context cookie survived across two sandbox workers");
    Ok(())
}
