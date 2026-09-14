use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use oryn_native::{Browser, ContextOptions};
use serde::Serialize;

const STREAM_BYTES_PER_PAGE: usize = 150_000;

#[derive(Debug, Serialize)]
struct PageMeasurement {
    page_id: u64,
    ready_ms: f64,
    stream_bytes: usize,
    stream_chunks: usize,
    websocket_echoed: bool,
    observation_bytes: usize,
}

#[derive(Debug, Serialize)]
struct BenchmarkResult {
    schema_version: u8,
    pages: usize,
    concurrent_ready_ms: f64,
    parent_rss_bytes: Option<usize>,
    worker_rss_bytes: Vec<usize>,
    total_worker_rss_bytes: usize,
    http_requests: usize,
    websocket_sessions: usize,
    stream_bytes_per_page: usize,
    page_measurements: Vec<PageMeasurement>,
    passed: bool,
}

fn serve_http(listener: TcpListener, expected: usize) -> usize {
    listener.set_nonblocking(true).expect("nonblocking HTTP");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut requests = Vec::with_capacity(expected);
    while requests.len() < expected && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                requests.push(thread::spawn(move || {
                    if stream.set_nonblocking(false).is_err() {
                        return false;
                    }
                    let mut request = [0_u8; 4096];
                    if stream.read(&mut request).is_err() {
                        return false;
                    }
                    let body = vec![b'x'; STREAM_BYTES_PER_PAGE];
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .and_then(|_| stream.write_all(&body))
                    .is_ok()
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
        }
    }
    requests
        .into_iter()
        .filter_map(|request| request.join().ok())
        .filter(|served| *served)
        .count()
}

fn serve_websockets(listener: TcpListener, expected: usize) -> usize {
    listener
        .set_nonblocking(true)
        .expect("nonblocking WebSocket");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut sessions = Vec::with_capacity(expected);
    while sessions.len() < expected && Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                sessions.push(thread::spawn(move || {
                    if stream.set_nonblocking(false).is_err() {
                        return false;
                    }
                    let Ok(mut socket) = tungstenite::accept(stream) else {
                        return false;
                    };
                    let Ok(message) = socket.read() else {
                        return false;
                    };
                    let sent = socket.send(message).is_ok();
                    let _ = socket.close(None);
                    sent
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
        }
    }
    sessions
        .into_iter()
        .filter_map(|session| session.join().ok())
        .filter(|served| *served)
        .count()
}

fn resident_bytes(pid: u32) -> Option<usize> {
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
        .flatten()
        .map(|kib: usize| kib.saturating_mul(1024))
}

fn child_pids(pid: u32) -> Vec<u32> {
    Command::new("/usr/bin/pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|value| value.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn page_html(http_port: u16, websocket_port: u16, index: usize) -> String {
    format!(
        r#"<!doctype html><title>Page-group page {index}</title>
        <output id=stream>pending</output><output id=socket>pending</output>
        <script>
          fetch('http://127.0.0.1:{http_port}/stream').then(async response=>{{
            const reader=response.body.getReader();let chunks=0,total=0;
            for(;;){{const item=await reader.read();if(item.done)break;chunks++;total+=item.value.length;}}
            document.querySelector('#stream').textContent=chunks+':'+total;
          }});
          const socket=new WebSocket('ws://127.0.0.1:{websocket_port}/echo');
          socket.onopen=()=>socket.send('page-{index}');
          socket.onmessage=event=>{{document.querySelector('#socket').textContent=event.data;socket.close();}};
        </script>"#
    )
}

fn exercise_page(
    page: oryn_native::PageHandle,
    http_port: u16,
    websocket_port: u16,
    index: usize,
) -> Result<PageMeasurement, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let started = Instant::now();
        page.load_html(
            format!("http://127.0.0.1:{http_port}/page-{index}"),
            page_html(http_port, websocket_port, index),
        )
        .await
        .map_err(|error| error.to_string())?;
        let mut state = String::new();
        for _ in 0..100 {
            state = page
                .evaluate(
                    "__oryn_drain(true);document.querySelector('#stream').textContent+'|'+document.querySelector('#socket').textContent",
                )
                .await
                .map_err(|error| error.to_string())?;
            if !state.contains("pending") {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        let (stream, socket) = state.split_once('|').unwrap_or_default();
        let (chunks, bytes) = stream
            .split_once(':')
            .and_then(|(chunks, bytes)| Some((chunks.parse().ok()?, bytes.parse().ok()?)))
            .unwrap_or_default();
        let observation = page.observe().await.map_err(|error| error.to_string())?;
        Ok(PageMeasurement {
            page_id: page.page_id().0,
            ready_ms: started.elapsed().as_secs_f64() * 1_000.0,
            stream_bytes: bytes,
            stream_chunks: chunks,
            websocket_echoed: socket == format!("page-{index}"),
            observation_bytes: serde_json::to_vec(&observation)
                .map_err(|error| error.to_string())?
                .len(),
        })
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pages = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(8_usize)
        .clamp(1, 20);
    let http_listener = TcpListener::bind("127.0.0.1:0")?;
    let http_port = http_listener.local_addr()?.port();
    let websocket_listener = TcpListener::bind("127.0.0.1:0")?;
    let websocket_port = websocket_listener.local_addr()?.port();
    let http_server = thread::spawn(move || serve_http(http_listener, pages));
    let websocket_server = thread::spawn(move || serve_websockets(websocket_listener, pages));

    let browser = Browser::new();
    let context = browser.new_context_with_options(ContextOptions::loopback_test());
    let handles = (0..pages).map(|_| context.new_page()).collect::<Vec<_>>();
    let concurrent_started = Instant::now();
    let workers = handles
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, page)| {
            thread::spawn(move || exercise_page(page, http_port, websocket_port, index))
        })
        .collect::<Vec<_>>();
    let mut page_measurements = workers
        .into_iter()
        .map(|worker| worker.join().map_err(|_| "page thread panicked")?)
        .collect::<Result<Vec<_>, _>>()
        .map_err(std::io::Error::other)?;
    page_measurements.sort_by_key(|measurement| measurement.page_id);
    let concurrent_ready_ms = concurrent_started.elapsed().as_secs_f64() * 1_000.0;

    let parent_pid = std::process::id();
    let mut worker_rss_bytes = child_pids(parent_pid)
        .into_iter()
        .filter_map(resident_bytes)
        .collect::<Vec<_>>();
    worker_rss_bytes.sort_unstable();
    let parent_rss_bytes = resident_bytes(parent_pid);
    let total_worker_rss_bytes = worker_rss_bytes.iter().sum();
    let http_requests = http_server.join().unwrap_or_default();
    let websocket_sessions = websocket_server.join().unwrap_or_default();
    let passed = http_requests == pages
        && websocket_sessions == pages
        && worker_rss_bytes.len() == pages
        && page_measurements.iter().all(|measurement| {
            measurement.stream_bytes == STREAM_BYTES_PER_PAGE
                && measurement.stream_chunks >= 3
                && measurement.websocket_echoed
        });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for page in handles {
        let _ = runtime.block_on(page.close());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&BenchmarkResult {
            schema_version: 1,
            pages,
            concurrent_ready_ms,
            parent_rss_bytes,
            worker_rss_bytes,
            total_worker_rss_bytes,
            http_requests,
            websocket_sessions,
            stream_bytes_per_page: STREAM_BYTES_PER_PAGE,
            page_measurements,
            passed,
        })?
    );
    if passed {
        Ok(())
    } else {
        Err("page-group benchmark failed".into())
    }
}
