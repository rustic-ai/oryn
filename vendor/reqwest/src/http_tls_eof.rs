//! Adapt rustls shutdown to HTTP's independently checked message framing.
use std::{io, task::Poll};

pub(crate) fn http_tls_read_result(result: Poll<io::Result<()>>) -> Poll<io::Result<()>> {
    // Normalize only rustls's exhausted-plaintext shutdown diagnostic. Handshake
    // EOF, corrupt TLS records and other I/O errors remain errors. Hyper still
    // rejects incomplete length/chunk framing and incomplete HTTP/2 streams.
    const SHUTDOWN_EOF: &str = "peer closed connection without sending TLS close_notify: \
https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof";
    match result {
        Poll::Ready(Err(error))
            if error.kind() == io::ErrorKind::UnexpectedEof
                && error
                    .get_ref()
                    .is_some_and(|cause| cause.to_string() == SHUTDOWN_EOF) =>
        {
            Poll::Ready(Ok(()))
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exhausted_plaintext_shutdown_is_http_eof() {
        let shutdown = io::Error::new(io::ErrorKind::UnexpectedEof,
            "peer closed connection without sending TLS close_notify: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof");
        assert!(matches!(
            http_tls_read_result(Poll::Ready(Err(shutdown))),
            Poll::Ready(Ok(()))
        ));
        for error in [
            io::Error::from(io::ErrorKind::UnexpectedEof),
            io::Error::new(io::ErrorKind::UnexpectedEof, "tls handshake eof"),
            io::Error::new(io::ErrorKind::InvalidData, "corrupt TLS record"),
            io::Error::from(io::ErrorKind::ConnectionReset),
            io::Error::new(io::ErrorKind::Other,
                "peer closed connection without sending TLS close_notify: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof"),
        ] {
            assert!(matches!(
                http_tls_read_result(Poll::Ready(Err(error))),
                Poll::Ready(Err(_))
            ));
        }
        assert!(matches!(http_tls_read_result(Poll::Pending), Poll::Pending));
    }
}
