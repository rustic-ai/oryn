//! HTTP admission shared by all driver routes; no session can bypass it.
use super::auth::DriverAuth;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_INFLIGHT_BODIES: usize = 8;

#[derive(Clone)]
struct Admission {
    auth: Arc<DriverAuth>,
    bodies: Arc<tokio::sync::Semaphore>,
}

fn rejected(status: StatusCode, message: &str) -> Response {
    (status,Json(serde_json::json!({"value":{"error":"invalid argument","message":message,"stacktrace":""}}))).into_response()
}

pub fn admits_headers(auth: &DriverAuth, headers: &HeaderMap) -> bool {
    let size = headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
        .sum::<usize>();
    if size > MAX_HEADER_BYTES {
        return false;
    }
    for name in [
        "host",
        "authorization",
        "origin",
        "content-length",
        "transfer-encoding",
    ] {
        if headers.get_all(name).iter().count() > 1 {
            return false;
        }
    }
    if headers.contains_key("content-length") && headers.contains_key("transfer-encoding") {
        return false;
    }
    // An invalid Origin header must not become equivalent to an absent one.
    if headers.contains_key("origin") {
        return false;
    }
    auth.permits(
        headers.get("host").and_then(|v| v.to_str().ok()),
        None,
        headers.get("authorization").and_then(|v| v.to_str().ok()),
    )
}

async fn admission(State(admission): State<Admission>, request: Request, next: Next) -> Response {
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || !admits_headers(&admission.auth, request.headers())
    {
        return rejected(StatusCode::FORBIDDEN, "driver request not authorized");
    }
    // Reserve before collecting any body. Keep the permit through dispatch so
    // slow native commands cannot accumulate an unbounded queue of full bodies.
    let Ok(_body_permit) = admission.bodies.clone().try_acquire_owned() else {
        return rejected(
            StatusCode::SERVICE_UNAVAILABLE,
            "driver request capacity exhausted",
        );
    };
    let (parts, body) = request.into_parts();
    let bytes =
        match tokio::time::timeout(Duration::from_secs(10), to_bytes(body, MAX_BODY_BYTES)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => {
                return rejected(StatusCode::BAD_REQUEST, "invalid or oversized command body");
            }
            Err(_) => {
                return rejected(
                    StatusCode::REQUEST_TIMEOUT,
                    "command body deadline exceeded",
                );
            }
        };
    let mut response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

pub fn secured(routes: Router, auth: Arc<DriverAuth>) -> Router {
    routes
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            Admission {
                auth,
                bodies: Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT_BODIES)),
            },
            admission,
        ))
}

#[cfg(unix)]
pub fn load_auth(address: SocketAddr, path: &Path) -> Result<DriverAuth, &'static str> {
    use std::{
        fs::OpenOptions,
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "cannot open driver credential file")?;
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect driver credential file")?;
    // SAFETY: geteuid takes no arguments and reads process identity only.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err("driver credential file must be private and owned by the current user");
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(65)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read driver credential file")?;
    let token = std::str::from_utf8(&bytes).map_err(|_| "invalid driver credential")?;
    DriverAuth::new(address, token)
}

#[cfg(not(unix))]
pub fn load_auth(_address: SocketAddr, _path: &Path) -> Result<DriverAuth, &'static str> {
    Err("private credential-file loading is not implemented on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("127.0.0.1:8123"));
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", "a".repeat(64))).unwrap(),
        );
        headers
    }
    #[tokio::test]
    async fn aggregate_admission_rejects_before_route_and_recovers_after_release() {
        use axum::{body::Body, routing::get};
        use tower::ServiceExt;
        let capacity = Arc::new(tokio::sync::Semaphore::new(1));
        let held = capacity.clone().acquire_owned().await.unwrap();
        let auth =
            Arc::new(DriverAuth::new("127.0.0.1:8123".parse().unwrap(), &"a".repeat(64)).unwrap());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let router = Router::new()
            .route(
                "/",
                get(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        "ok"
                    }
                }),
            )
            .layer(middleware::from_fn_with_state(
                Admission {
                    auth,
                    bodies: capacity,
                },
                admission,
            ));
        let request = || {
            let mut request = Request::builder().uri("/").body(Body::empty()).unwrap();
            *request.headers_mut() = headers();
            request
        };
        let response = router.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(held);
        assert_eq!(
            router.oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn rejects_duplicate_headers_and_ambiguous_message_framing() {
        let auth = DriverAuth::new("127.0.0.1:8123".parse().unwrap(), &"a".repeat(64)).unwrap();
        assert!(admits_headers(&auth, &headers()));
        for name in [
            "host",
            "authorization",
            "content-length",
            "transfer-encoding",
        ] {
            let mut value = headers();
            value.append(name, HeaderValue::from_static("one"));
            value.append(name, HeaderValue::from_static("two"));
            assert!(!admits_headers(&auth, &value));
        }
        let mut value = headers();
        value.insert("content-length", HeaderValue::from_static("0"));
        value.insert("transfer-encoding", HeaderValue::from_static("chunked"));
        assert!(!admits_headers(&auth, &value));
        let mut value = headers();
        value.insert("origin", HeaderValue::from_bytes(&[255]).unwrap());
        assert!(!admits_headers(&auth, &value));
    }
    #[tokio::test]
    async fn authentication_precedes_dispatch_and_body_limits_apply() {
        use axum::{body::Bytes, routing::post};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tower::ServiceExt;
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let routes = Router::new().route(
            "/session",
            post(move |body: Bytes| {
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    body.len().to_string()
                }
            }),
        );
        let auth =
            Arc::new(DriverAuth::new("127.0.0.1:8123".parse().unwrap(), &"a".repeat(64)).unwrap());
        let app = secured(routes, auth);
        let request = Request::builder()
            .method("POST")
            .uri("/session")
            .header("host", "127.0.0.1:8123")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let request = |body: Body| {
            Request::builder()
                .method("POST")
                .uri("/session")
                .header("host", "127.0.0.1:8123")
                .header("authorization", format!("Bearer {}", "a".repeat(64)))
                .body(body)
                .unwrap()
        };
        let response = app
            .clone()
            .oneshot(request(Body::from("{}")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            app.oneshot(request(Body::from(vec![0; MAX_BODY_BYTES + 1])))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[test]
    fn credential_loader_rejects_public_permissions_and_symlinks() {
        use std::{
            fs::{OpenOptions, Permissions},
            io::Write,
            os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
        };
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let name = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = std::env::temp_dir().join(format!("oryn-driver-test-{name}"));
        let link = path.with_extension("link");
        struct Cleanup(std::path::PathBuf, std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
                let _ = std::fs::remove_file(&self.1);
            }
        }
        let _cleanup = Cleanup(path.clone(), link.clone());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all("a".repeat(64).as_bytes()).unwrap();
        drop(file);
        let address = "127.0.0.1:8123".parse().unwrap();
        assert!(load_auth(address, &path).is_ok());
        symlink(&path, &link).unwrap();
        assert!(load_auth(address, &link).is_err());
        std::fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        assert!(load_auth(address, &path).is_err());
    }
}
