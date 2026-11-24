//! A reader that does not accept a bare LF in the head of a request.
//!
//! hyper (through httparse) takes a lone `\n` as the end of a line. A proxy in front of the hub that
//! reads the same bytes as one line (`X-Note: a\nX-Smuggled: 1`) and the hub that reads two headers
//! disagree about the message, which is how a request is smuggled. The hub reads the way the strictest
//! reader does: every line of a head ends in CRLF.
//!
//! [`StrictHead`] sits between the socket and hyper. It follows the messages of the connection (head,
//! then body by `Content-Length` or chunked, then the next head) and replaces a bare LF inside a head
//! by a NUL byte, which hyper refuses: the answer is 400 and the connection closes. Bodies pass
//! untouched, they may hold any byte.
//!
//! When the framing of a message is unclear (a `Content-Length` that is not plain digits, two that
//! differ, `Transfer-Encoding` with anything but `chunked`, both headers) the reader stops following:
//! hyper refuses exactly those messages itself with 400 and closes, so nothing behind them is read.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The longest line kept to read its header name; a longer line is hyper's to refuse (`max_buf_size`).
const LINE_CAP: usize = 8 * 1024;

/// Where the reader is in the byte stream of the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Inside a head: the bytes of the current line are collected.
    Head,
    /// A fixed number of body bytes are left.
    Body(u64),
    /// The size line of a chunk (hex digits, maybe extensions) until its CRLF.
    ChunkSize,
    /// The data of a chunk and then its CRLF.
    ChunkData(u64),
    /// The CRLF after the data of a chunk, `0` is how many of the two bytes were seen.
    ChunkEnd(u8),
    /// The trailer after the last chunk: lines until an empty one.
    Trailer,
    /// The framing is unclear: not followed any more (hyper refuses the message).
    Lost,
}

/// What the head collected so far says about its body.
#[derive(Debug, Default, Clone)]
struct Framing {
    length: Option<u64>,
    chunked: bool,
    other_encoding: bool,
    unclear: bool,
}

/// The state machine; separate from the IO so that it is tested on plain bytes.
#[derive(Debug, Clone)]
pub struct HeadGuard {
    state: State,
    line: Vec<u8>,
    prev_cr: bool,
    first_line: bool,
    framing: Framing,
}

impl Default for HeadGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadGuard {
    pub fn new() -> Self {
        Self {
            state: State::Head,
            line: Vec::new(),
            prev_cr: false,
            first_line: true,
            framing: Framing::default(),
        }
    }

    /// Looks at the next bytes of the stream and replaces a bare LF of a head by NUL.
    pub fn scan(&mut self, bytes: &mut [u8]) {
        let mut i = 0;
        while i < bytes.len() {
            match self.state {
                State::Lost => {
                    // Fail closed: what follows a message that cannot be framed is not read as a message
                    // (hyper refuses NUL and closes), so a bare LF cannot hide behind a framing the reader
                    // lost but hyper accepted.
                    bytes[i..].fill(0);
                    return;
                }
                State::Body(left) => {
                    let n = usize::try_from(left)
                        .unwrap_or(usize::MAX)
                        .min(bytes.len() - i);
                    i += n;
                    let left = left - n as u64;
                    self.state = if left == 0 {
                        self.next_head()
                    } else {
                        State::Body(left)
                    };
                }
                State::ChunkData(left) => {
                    let n = usize::try_from(left)
                        .unwrap_or(usize::MAX)
                        .min(bytes.len() - i);
                    i += n;
                    let left = left - n as u64;
                    self.state = if left == 0 {
                        State::ChunkEnd(0)
                    } else {
                        State::ChunkData(left)
                    };
                }
                State::ChunkEnd(seen) => {
                    let want = if seen == 0 { b'\r' } else { b'\n' };
                    if bytes[i] != want {
                        // hyper refuses a chunk that does not end in CRLF.
                        self.state = State::Lost;
                        continue;
                    }
                    i += 1;
                    self.state = if seen == 0 {
                        State::ChunkEnd(1)
                    } else {
                        State::ChunkSize
                    };
                }
                State::Head | State::ChunkSize | State::Trailer => {
                    let b = bytes[i];
                    i += 1;
                    // A bare LF ends here in `Lost`; the next turn of the loop clears the rest.
                    self.line_byte(b, &mut bytes[i - 1]);
                }
            }
        }
    }

    /// The state after a message is complete: the next head.
    fn next_head(&mut self) -> State {
        self.line.clear();
        self.prev_cr = false;
        self.first_line = true;
        self.framing = Framing::default();
        State::Head
    }

    /// One byte of a line (head, chunk size, or trailer).
    fn line_byte(&mut self, b: u8, slot: &mut u8) {
        if b == b'\n' {
            if !self.prev_cr {
                // A lone LF: hyper reads it as a line end, a stricter reader does not.
                if self.state == State::Head {
                    *slot = 0;
                }
                // In a chunk size or trailer hyper's own parser decides; both refuse a bare LF in a size.
                self.state = State::Lost;
                return;
            }
            self.prev_cr = false;
            self.end_of_line();
            return;
        }
        self.prev_cr = b == b'\r';
        if self.line.len() < LINE_CAP {
            self.line.push(b);
        }
    }

    /// A line ended in CRLF; `self.line` holds it with the CR.
    fn end_of_line(&mut self) {
        let mut line = std::mem::take(&mut self.line);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        match self.state {
            State::Head => {
                if line.is_empty() {
                    // A blank line before the request line is skipped by hyper; after it, it ends the head.
                    if self.first_line {
                        return;
                    }
                    self.state = self.body_state();
                } else {
                    if !self.first_line {
                        self.header_line(&line);
                    }
                    self.first_line = false;
                }
            }
            State::ChunkSize => match chunk_size(&line) {
                Some(0) => self.state = State::Trailer,
                Some(n) => self.state = State::ChunkData(n),
                None => self.state = State::Lost,
            },
            State::Trailer if line.is_empty() => self.state = self.next_head(),
            _ => {}
        }
    }

    fn header_line(&mut self, line: &[u8]) {
        let Some(colon) = line.iter().position(|&c| c == b':') else {
            self.framing.unclear = true;
            return;
        };
        let (name, value) = (&line[..colon], trim(&line[colon + 1..]));
        if name.eq_ignore_ascii_case(b"content-length") {
            let digits = !value.is_empty() && value.iter().all(u8::is_ascii_digit);
            let parsed = std::str::from_utf8(value)
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            match (digits, parsed, self.framing.length) {
                (true, Some(n), None) => self.framing.length = Some(n),
                (true, Some(n), Some(old)) if n == old => {}
                _ => self.framing.unclear = true,
            }
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            if value.eq_ignore_ascii_case(b"chunked") && !self.framing.chunked {
                self.framing.chunked = true;
            } else {
                self.framing.other_encoding = true;
            }
        }
    }

    /// What follows the blank line that ended the head.
    fn body_state(&mut self) -> State {
        let f = self.framing.clone();
        if f.unclear || f.other_encoding || (f.chunked && f.length.is_some()) {
            return State::Lost;
        }
        if f.chunked {
            self.line.clear();
            self.prev_cr = false;
            return State::ChunkSize;
        }
        match f.length {
            Some(n) if n > 0 => State::Body(n),
            _ => self.next_head(),
        }
    }
}

/// The hex size of a chunk; extensions after `;` are ignored.
fn chunk_size(line: &[u8]) -> Option<u64> {
    let end = line.iter().position(|&c| c == b';').unwrap_or(line.len());
    let digits = trim(&line[..end]);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u64::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|c| !matches!(c, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|c| !matches!(c, b' ' | b'\t'))
        .map_or(start, |p| p + 1);
    &bytes[start..end]
}

/// A stream whose reads pass through a [`HeadGuard`].
pub struct StrictHead<T> {
    inner: T,
    guard: HeadGuard,
}

impl<T> StrictHead<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            guard: HeadGuard::new(),
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for StrictHead<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let this = &mut *self;
        let polled = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = polled {
            this.guard.scan(&mut buf.filled_mut()[before..]);
        }
        polled
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for StrictHead<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the bytes through the guard in pieces of `step` and returns what hyper would read.
    fn scanned(input: &[u8], step: usize) -> Vec<u8> {
        let mut guard = HeadGuard::new();
        let mut out = input.to_vec();
        for piece in out.chunks_mut(step) {
            guard.scan(piece);
        }
        out
    }

    fn all_steps(input: &[u8]) -> Vec<Vec<u8>> {
        [1, 2, 3, 7, input.len()]
            .iter()
            .map(|&s| scanned(input, s))
            .collect()
    }

    /// What hyper reads when the reader refuses at byte `cut`: everything before it as sent, the rest NUL.
    fn refused_at(input: &[u8], cut: usize) -> Vec<u8> {
        let mut out = input[..cut].to_vec();
        out.resize(input.len(), 0);
        out
    }

    #[test]
    fn a_bare_lf_in_a_header_is_refused() {
        let input = b"GET / HTTP/1.1\r\nX-Note: a\nX-Smuggled: 1\r\n\r\n";
        let cut = b"GET / HTTP/1.1\r\nX-Note: a".len();
        for out in all_steps(input) {
            assert_eq!(out, refused_at(input, cut));
        }
    }

    #[test]
    fn a_bare_lf_in_the_request_line_is_refused() {
        let input = b"GET / HTTP/1.1\nHost: x\r\n\r\n";
        for out in all_steps(input) {
            assert_eq!(out, refused_at(input, b"GET / HTTP/1.1".len()));
        }
    }

    #[test]
    fn a_proper_head_and_a_blank_line_before_it_pass() {
        let input = b"\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\nGET /b HTTP/1.1\r\nHost: x\r\n\r\n";
        for out in all_steps(input) {
            assert_eq!(out, input);
        }
    }

    #[test]
    fn a_body_with_bare_lf_passes_and_the_next_head_is_checked() {
        let input = b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\na\nb\nc\
                      GET / HTTP/1.1\r\nX: a\nb\r\n\r\n";
        let cut = input.len() - b"\nb\r\n\r\n".len();
        for out in all_steps(input) {
            assert_eq!(out, refused_at(input, cut));
        }
    }

    #[test]
    fn a_chunked_body_with_bare_lf_passes_and_the_next_head_is_checked() {
        let input = b"POST / HTTP/1.1\r\ntransfer-encoding: Chunked\r\n\r\n3\r\na\nb\r\n2;ext=1\r\n\n\n\r\n0\r\nT: v\r\n\r\n\
                      GET / HTTP/1.1\r\nX: a\nb\r\n\r\n";
        let cut = input.len() - b"\nb\r\n\r\n".len();
        for out in all_steps(input) {
            assert_eq!(out, refused_at(input, cut));
        }
    }

    #[test]
    fn unclear_framing_stops_the_following() {
        // hyper refuses these itself; the reader must not guess a body length for them.
        for head in [
            &b"POST / HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 9\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nContent-Length: +2\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip, chunked\r\n\r\n"[..],
        ] {
            let mut guard = HeadGuard::new();
            let mut bytes = head.to_vec();
            guard.scan(&mut bytes);
            assert_eq!(
                guard.state,
                State::Lost,
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    #[test]
    fn after_unclear_framing_nothing_passes_that_could_hide_a_bare_lf() {
        // hyper may accept a message the reader cannot frame (here two Transfer-Encoding lines). The
        // reader must not then wave through whatever follows: a bare LF behind it would be read as two
        // headers by hyper and as one by a proxy.
        let head =
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n\r\n";
        let rest = b"0\r\n\r\nGET / HTTP/1.1\r\nX: a\nX-Smuggled: 1\r\n\r\n";
        for step in [1, 5, 1000] {
            let mut input = head.to_vec();
            input.extend_from_slice(rest);
            let out = scanned(&input, step);
            assert_eq!(&out[..head.len()], head, "step {step}");
            assert!(
                !out[head.len()..].contains(&b'\n'),
                "step {step}: {:?}",
                String::from_utf8_lossy(&out)
            );
        }
    }

    #[test]
    fn equal_content_lengths_are_one_length() {
        let mut guard = HeadGuard::new();
        let mut bytes =
            b"POST / HTTP/1.1\r\nContent-Length: 2\r\ncontent-length: 2\r\n\r\n".to_vec();
        guard.scan(&mut bytes);
        assert_eq!(guard.state, State::Body(2));
    }

    #[tokio::test]
    async fn the_reader_changes_what_it_reads_from_a_socket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut client, server) = tokio::io::duplex(64);
        let mut strict = StrictHead::new(server);
        client
            .write_all(b"GET / HTTP/1.1\r\nA: b\nC: d\r\n\r\n")
            .await
            .unwrap();
        let sent = b"GET / HTTP/1.1\r\nA: b\nC: d\r\n\r\n";
        let expected = refused_at(sent, b"GET / HTTP/1.1\r\nA: b".len());
        let mut got = vec![0u8; expected.len()];
        // A wrong length must fail the test, not hang it.
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            strict.read_exact(&mut got),
        )
        .await
        .expect("the bytes arrive")
        .unwrap();
        assert_eq!(got, expected);
    }
}
