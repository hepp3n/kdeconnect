use pin_project::pin_project;
use std::ops::Div;
use tokio::{
    io::AsyncRead,
    sync::mpsc::{self},
    time::{Duration, Interval, interval},
};

use crate::event::ConnectionEvent;

#[pin_project]
pub(crate) struct TransferAdapter<R: AsyncRead> {
    #[pin]
    inner: R,
    transfer_interval: Interval,
    transfer_bytes: usize,
    total_size: u64,
    processed_percent: u8,
    pub(crate) notify_tx: mpsc::UnboundedSender<ConnectionEvent>,
}

impl<R: AsyncRead> TransferAdapter<R> {
    pub fn new(
        inner: R,
        total_size: u64,
        connection_tx: mpsc::UnboundedSender<ConnectionEvent>,
    ) -> Self {
        Self {
            inner,
            transfer_interval: interval(Duration::from_millis(100)),
            transfer_bytes: 0,
            total_size,
            processed_percent: 0,
            notify_tx: connection_tx,
        }
    }
}

impl<R: AsyncRead> AsyncRead for TransferAdapter<R> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.project();
        let before = buf.filled().len();
        let result = this.inner.poll_read(cx, buf);
        let filled_len = buf.filled().len() - before;

        *this.transfer_bytes += filled_len;
        *this.processed_percent =
            calculate_progress(*this.transfer_bytes as f64, *this.total_size as f64);

        // Emit 100% immediately on EOF (zero-byte read) so small/fast
        // transfers always surface a completion update regardless of the
        // 100 ms ticker cadence.
        if matches!(result, std::task::Poll::Ready(Ok(()))) && filled_len == 0 {
            send_progress(100, this.notify_tx.clone());
            return result;
        }

        match this.transfer_interval.poll_tick(cx) {
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(_) => {
                send_progress(*this.processed_percent, this.notify_tx.clone());
            }
        }

        result
    }
}

fn calculate_progress(transferred: f64, total: f64) -> u8 {
    if total > 0.0 && transferred > 0.0 {
        (transferred.div(total) * 100.0).round().min(100.0) as u8
    } else {
        0
    }
}

pub(crate) fn send_progress(percent: u8, notify_tx: mpsc::UnboundedSender<ConnectionEvent>) {
    let _ = notify_tx.send(ConnectionEvent::UpdateTransferProgress(percent));
}
