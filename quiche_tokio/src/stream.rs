use super::connection;
use std::future::Future;

#[derive(Copy, Clone)]
pub struct StreamID(pub(super) u64);

impl std::fmt::Debug for StreamID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "StreamID({}, bidi={}, {})",
            self.stream_id(),
            self.is_bidi(),
            if self.is_server() { "server" } else { "client" }
        ))
    }
}
impl StreamID {
    pub fn new(stream_id: u64, bidi: bool, is_server: bool) -> Self {
        let client_flag = if is_server { 1 } else { 0 };
        let bidi_flag = if bidi { 0 } else { 2 };
        Self(stream_id << 2 | bidi_flag | client_flag)
    }

    pub fn full_stream_id(&self) -> u64 {
        self.0
    }

    pub fn stream_id(&self) -> u64 {
        self.0 >> 2
    }

    pub fn is_server(&self) -> bool {
        self.0 & 1 == 1
    }

    pub fn is_bidi(&self) -> bool {
        self.0 & 2 == 0
    }

    pub fn can_read(&self, is_server: bool) -> bool {
        self.is_bidi() || (is_server && (self.0 & 1 == 0)) || (!is_server && (self.0 & 1 == 1))
    }

    pub fn can_write(&self, is_server: bool) -> bool {
        self.is_bidi() || (is_server && (self.0 & 1 == 1)) || (!is_server && (self.0 & 1 == 0))
    }
}

type ReadOutput = std::io::Result<Vec<u8>>;
type WriteOutput = std::io::Result<usize>;
type StreamFut<T> = Option<std::pin::Pin<Box<dyn Future<Output = T> + Send + Sync>>>;
pub struct Stream {
    is_server: bool,
    stream_id: StreamID,
    shared_state: std::sync::Arc<connection::SharedConnectionState>,
    control_tx: tokio::sync::mpsc::Sender<connection::Control>,
    async_read: StreamFut<ReadOutput>,
    async_write: StreamFut<WriteOutput>,
    async_shutdown: StreamFut<WriteOutput>,
    read_fin: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Stream {
    pub(crate) fn new(
        is_server: bool,
        stream_id: StreamID,
        shared_state: std::sync::Arc<connection::SharedConnectionState>,
        control_tx: tokio::sync::mpsc::Sender<connection::Control>,
    ) -> Self {
        Self {
            is_server,
            stream_id,
            shared_state,
            control_tx,
            async_read: None,
            async_write: None,
            async_shutdown: None,
            read_fin: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn stream_id(&self) -> StreamID {
        self.stream_id
    }
}

impl Clone for Stream {
    fn clone(&self) -> Self {
        Self {
            is_server: self.is_server,
            stream_id: self.stream_id,
            shared_state: self.shared_state.clone(),
            control_tx: self.control_tx.clone(),
            async_read: None,
            async_write: None,
            async_shutdown: None,
            read_fin: self.read_fin.clone(),
        }
    }
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("is_server", &self.is_server)
            .field("stream_id", &self.stream_id)
            .field("shared_state", &self.shared_state)
            .finish_non_exhaustive()
    }
}

impl Stream {
    pub fn is_bidi(&self) -> bool {
        self.stream_id.is_bidi()
    }

    pub fn can_read(&self) -> bool {
        self.stream_id.can_read(self.is_server)
    }

    pub fn can_write(&self) -> bool {
        self.stream_id.can_write(self.is_server)
    }

    async fn _read(
        stream_id: StreamID,
        shared_state: std::sync::Arc<connection::SharedConnectionState>,
        control_tx: tokio::sync::mpsc::Sender<connection::Control>,
        len: usize,
        read_fin: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> ReadOutput {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if control_tx
            .send(connection::Control::StreamRecv {
                stream_id: stream_id.0,
                len,
                resp: tx,
            })
            .await
            .is_err()
        {
            match shared_state.connection_error.read().await.clone() {
                Some(err) => {
                    trace!("Connection error: {:?}", err);
                    return Err(std::io::Error::other(err));
                }
                None => {
                    read_fin.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Ok(vec![]);
                }
            }
        }
        match rx.await {
            Ok(Ok((r, fin))) => {
                if fin {
                    read_fin.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(r)
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => {
                match shared_state.connection_error.read().await.clone() {
                    Some(err) => {
                        trace!("Connection error: {:?}", err);
                        return Err(std::io::Error::other(err));
                    }
                    None => {
                        read_fin.store(true, std::sync::atomic::Ordering::Relaxed);
                        return Ok(vec![]);
                    }
                }
            }
        }
    }

    async fn _write(
        stream_id: StreamID,
        shared_state: std::sync::Arc<connection::SharedConnectionState>,
        control_tx: tokio::sync::mpsc::Sender<connection::Control>,
        data: Vec<u8>,
        fin: bool,
    ) -> WriteOutput {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if control_tx
            .send(connection::Control::StreamSend {
                stream_id: stream_id.0,
                data,
                fin,
                resp: tx,
            })
            .await
            .is_err()
        {
            match shared_state.connection_error.read().await.clone() {
                Some(err) => {
                    trace!("Connection error: {:?}", err);
                    return Err(std::io::Error::other(err));
                }
                None => {
                    return Ok(0);
                }
            }
        }
        match rx.await {
            Ok(r) => r.map_err(|e| e.into()),
            Err(_) => {
                match shared_state.connection_error.read().await.clone() {
                    Some(err) => {
                        trace!("Connection error: {:?}", err);
                        return Err(std::io::Error::other(err));
                    }
                    None => {
                        return Ok(0);
                    }
                }
            }
        }
    }
}

impl tokio::io::AsyncRead for Stream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.can_read() {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Write-only stream",
            )));
        }
        if self.read_fin.load(std::sync::atomic::Ordering::Acquire) {
            return std::task::Poll::Ready(Ok(()));
        }
        if let Some(fut) = &mut self.async_read {
            return match fut.as_mut().poll(cx) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(Ok(r)) => {
                    self.async_read = None;
                    buf.put_slice(&r);
                    std::task::Poll::Ready(Ok(()))
                }
                std::task::Poll::Ready(Err(e)) => {
                    self.async_read = None;
                    std::task::Poll::Ready(Err(e))
                }
            };
        }
        let mut fut = Box::pin(Self::_read(
            self.stream_id,
            self.shared_state.clone(),
            self.control_tx.clone(),
            buf.remaining(),
            self.read_fin.clone(),
        ));
        match fut.as_mut().poll(cx) {
            std::task::Poll::Pending => {
                self.async_read.replace(fut);
                std::task::Poll::Pending
            }
            std::task::Poll::Ready(Ok(r)) => {
                buf.put_slice(&r);
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e)),
        }
    }
}

impl tokio::io::AsyncWrite for Stream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if !self.can_write() {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Read-only stream",
            )));
        }
        if let Some(fut) = &mut self.async_write {
            return match fut.as_mut().poll(cx) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(r) => {
                    self.async_write = None;
                    std::task::Poll::Ready(r)
                }
            };
        }
        let mut fut = Box::pin(Self::_write(
            self.stream_id,
            self.shared_state.clone(),
            self.control_tx.clone(),
            buf.to_vec(),
            false,
        ));
        match fut.as_mut().poll(cx) {
            std::task::Poll::Pending => {
                self.async_write.replace(fut);
                std::task::Poll::Pending
            }
            std::task::Poll::Ready(r) => std::task::Poll::Ready(r),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Some(fut) = &mut self.async_shutdown {
            return match fut.as_mut().poll(cx) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(r) => {
                    self.async_shutdown = None;
                    std::task::Poll::Ready(r.map(|_| ()))
                }
            };
        }
        let mut fut = Box::pin(Self::_write(
            self.stream_id,
            self.shared_state.clone(),
            self.control_tx.clone(),
            Vec::new(),
            true,
        ));
        match fut.as_mut().poll(cx) {
            std::task::Poll::Pending => {
                self.async_shutdown.replace(fut);
                std::task::Poll::Pending
            }
            std::task::Poll::Ready(r) => std::task::Poll::Ready(r.map(|_| ())),
        }
    }
}
