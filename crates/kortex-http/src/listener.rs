//! Admission applies before reading HTTP headers too: idle sockets cannot grow without bound.
use axum::serve::Listener;
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Sleep, sleep},
};

pub(crate) struct BoundedListener {
    listener: TcpListener,
    slots: Arc<Semaphore>,
    lifetime: Duration,
}
impl BoundedListener {
    pub(crate) fn new(listener: TcpListener, connections: usize, lifetime: Duration) -> Self {
        Self {
            listener,
            slots: Arc::new(Semaphore::new(connections)),
            lifetime,
        }
    }
}
pub(crate) struct Connection {
    stream: TcpStream,
    _permit: OwnedSemaphorePermit,
    expires: Pin<Box<Sleep>>,
}
impl Connection {
    fn expired(&mut self, cx: &mut Context<'_>) -> bool {
        self.expires.as_mut().poll(cx).is_ready()
    }
}
impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.expired(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP connection lifetime exceeded",
            )));
        }
        Pin::new(&mut this.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.expired(cx) {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP connection lifetime exceeded",
            )));
        }
        Pin::new(&mut this.stream).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
impl Listener for BoundedListener {
    type Io = Connection;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let permit = match self.slots.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            match self.listener.accept().await {
                Ok((stream, address)) => {
                    let _ = stream.set_nodelay(true);
                    return (
                        Connection {
                            stream,
                            _permit: permit,
                            expires: Box::pin(sleep(self.lifetime)),
                        },
                        address,
                    );
                }
                Err(_) => {
                    drop(permit);
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn incomplete_headers_have_a_hard_connection_lifetime() {
        use tokio::io::AsyncReadExt;
        let raw = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = raw.local_addr().unwrap();
        let mut listener = BoundedListener::new(raw, 1, Duration::from_millis(5));
        let _idle = TcpStream::connect(address).await.unwrap();
        let (mut connection, _) = listener.accept().await;
        assert_eq!(listener.slots.available_permits(), 0);
        let mut byte = [0];
        let error = connection.read(&mut byte).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(connection);
        assert_eq!(listener.slots.available_permits(), 1);
    }
}
