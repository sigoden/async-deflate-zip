use std::borrow::Cow;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::AsyncWrite;

/// Counts bytes written through it.
pub(crate) struct CountWriter<W> {
    pub(crate) inner: W,
    pub(crate) count: u64,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for CountWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                this.count += n as u64;
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Normalize a ZIP entry path: replace backslashes with forward slashes
/// and ensure directories end with `/`.
pub(crate) fn sanitize_path(name: &str, is_directory: bool) -> Cow<'_, str> {
    let sanitized = if name.contains('\\') {
        Cow::Owned(name.replace('\\', "/"))
    } else {
        Cow::Borrowed(name)
    };
    if is_directory && !sanitized.ends_with('/') {
        Cow::Owned(format!("{sanitized}/"))
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_count_writer_counts_bytes() {
        let buf = Vec::new();
        let mut cw = CountWriter {
            inner: buf,
            count: 0,
        };
        cw.write_all(b"hello").await.unwrap();
        cw.write_all(b" ").await.unwrap();
        cw.write_all(b"world").await.unwrap();
        let count = cw.count;
        let inner = &cw.inner;
        assert_eq!(count, 11, "CountWriter should track total bytes written");
        assert_eq!(inner, b"hello world", "data should pass through unchanged");
    }
}
