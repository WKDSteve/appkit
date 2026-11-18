//! A tiny HTTP/1.1 client for the two things the hub's own programs do:
//! ask a running hub who it is, and ask it for a pairing code.
//!
//! It speaks to `127.0.0.1` only, sends `Connection: close`, and reads at
//! most 64 KiB within 5 s ([`request`]); a caller that expects a larger
//! reply names its own bounds ([`request_with`], [`ReadLimits`]). There is
//! no TLS, no redirect, no proxy: nothing a program that sends a credential
//! to a local port should have.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How much of a reply the client reads, and how long the whole exchange
/// may take. [`ReadLimits::default`] is what [`request`] uses: 64 KiB, 5 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadLimits {
    /// The most bytes of the reply (head and body) read; more is an error.
    pub max_reply_bytes: usize,
    /// The time for connecting, sending and reading the whole reply.
    pub timeout: Duration,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            max_reply_bytes: 64 * 1024,
            timeout: Duration::from_secs(5),
        }
    }
}

/// A status code, the headers and the body text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    /// The value of a header, by name without regard to case.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Send one request to the hub on `port`, read at most 64 KiB within 5 s.
///
/// # Errors
///
/// I/O errors, a time-out, a reply over the size limit or not HTTP.
pub async fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> io::Result<Reply> {
    request_with(port, method, path, headers, body, ReadLimits::default()).await
}

/// [`request`] with bounds of the caller's own: for a reply that may be
/// larger than 64 KiB (a job of ai-desktop) or slower than 5 s.
///
/// # Errors
///
/// I/O errors, a time-out, a reply over `limits.max_reply_bytes` or not HTTP.
pub async fn request_with(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    limits: ReadLimits,
) -> io::Result<Reply> {
    // The request line is built from these two: neither may carry a space
    // or a control character, or it would be a second line or a second part.
    let plain = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_graphic());
    if !plain(method) || !plain(path) || !path.starts_with('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the method or the path is not plain text",
        ));
    }
    let mut text = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (name, value) in headers {
        // A header value with a line break would be a second header.
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a header holds a line break",
            ));
        }
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));

    let exchange = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(text.as_bytes()).await?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            if buf.len() + n > limits.max_reply_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the reply is larger than expected",
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok::<_, io::Error>(buf)
    };
    let bytes = tokio::time::timeout(limits.timeout, exchange)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the hub did not answer"))??;
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> io::Result<Reply> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "not an HTTP reply");
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(bad)?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .filter(|l| l.starts_with("HTTP/1."))
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(bad)?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Ok(Reply {
        status,
        headers,
        body: body.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_parsed() {
        let r = parse(b"HTTP/1.1 200 OK\r\nETag: \"x\"\r\ncontent-length: 2\r\n\r\n{}").unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "{}");
        assert_eq!(r.header("etag"), Some("\"x\""));
        assert_eq!(r.header("missing"), None);
    }

    #[test]
    fn nonsense_is_not_a_reply() {
        assert!(parse(b"").is_err());
        assert!(parse(b"hello").is_err());
        assert!(parse(b"HTTP/1.1 abc\r\n\r\n").is_err());
        assert!(parse(&[0xff, 0xfe, b'\r', b'\n', b'\r', b'\n']).is_err());
        assert!(parse(b"SSH-2.0 200\r\n\r\n").is_err(), "not HTTP/1.x");
        assert!(parse(b"HTTP/1.1 200 OK\r\n\r\n\xff").is_err(), "not UTF-8");
    }

    /// A server on the loopback that answers one request with `body` (200),
    /// after `delay`.
    async fn serve_once(body: String, delay: Duration) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = [0u8; 4096];
            let _ = stream.read(&mut head).await;
            tokio::time::sleep(delay).await;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
        });
        port
    }

    #[tokio::test]
    async fn the_default_bounds_are_64_kib_and_5_s() {
        assert_eq!(
            ReadLimits::default(),
            ReadLimits {
                max_reply_bytes: 64 * 1024,
                timeout: Duration::from_secs(5),
            }
        );
        let port = serve_once("x".repeat(100 * 1024), Duration::ZERO).await;
        let err = request(port, "GET", "/", &[], "").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_reply_over_64_kib_is_read_whole_within_a_larger_limit() {
        let body = "x".repeat(700 * 1024);
        let port = serve_once(body.clone(), Duration::ZERO).await;
        let limits = ReadLimits {
            max_reply_bytes: 2 * 1024 * 1024,
            ..ReadLimits::default()
        };
        let reply = request_with(port, "GET", "/jobs/a1", &[], "", limits)
            .await
            .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, body);
    }

    #[tokio::test]
    async fn a_reply_over_the_limit_is_refused() {
        let port = serve_once("x".repeat(300 * 1024), Duration::ZERO).await;
        let limits = ReadLimits {
            max_reply_bytes: 256 * 1024,
            ..ReadLimits::default()
        };
        let err = request_with(port, "GET", "/", &[], "", limits)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_reply_slower_than_the_limit_is_a_time_out() {
        let port = serve_once("{}".to_owned(), Duration::from_secs(2)).await;
        let limits = ReadLimits {
            timeout: Duration::from_millis(200),
            ..ReadLimits::default()
        };
        let err = request_with(port, "GET", "/", &[], "", limits)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn a_method_or_path_with_a_break_or_a_space_is_refused() {
        for (method, path) in [
            ("GET", "/a b"),
            ("GET", "/a\r\nHost: x"),
            ("GET", "a"),
            ("GET", ""),
            ("GE T", "/"),
            ("", "/"),
        ] {
            let err = request(1, method, path, &[], "").await.unwrap_err();
            assert_eq!(
                err.kind(),
                io::ErrorKind::InvalidInput,
                "{method:?} {path:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_header_with_a_line_break_is_refused() {
        let err = request(1, "GET", "/", &[("X", "a\r\nInjected: 1")], "")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
