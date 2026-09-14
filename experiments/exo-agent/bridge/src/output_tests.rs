// SPDX-License-Identifier: MIT

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;

#[derive(Default)]
struct Buffered {
    accepted: Vec<u8>,
    published: Vec<u8>,
    polled: bool,
    fail_flush: bool,
}

impl AsyncWrite for Buffered {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.accepted.extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.polled {
            self.polled = true;
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        if self.fail_flush {
            return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
        }
        let accepted = std::mem::take(&mut self.accepted);
        self.published.extend(accepted);
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(context)
    }
}

#[tokio::test]
async fn accepted_bytes_are_not_success_until_publication_completes() {
    let mut writer = Buffered::default();
    assert!(
        super::publish(&mut writer, b"synthetic receipt")
            .await
            .is_ok()
    );
    assert_eq!(writer.published, b"synthetic receipt");
    assert!(writer.accepted.is_empty());

    let mut failed = Buffered {
        fail_flush: true,
        ..Buffered::default()
    };
    assert_eq!(
        super::publish(&mut failed, b"synthetic receipt").await,
        Err("exo_executor_output")
    );
    assert!(failed.published.is_empty());
}
