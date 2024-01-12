pub struct UdpSocket {
    sock: tokio::net::UdpSocket
}

impl UdpSocket {
    pub async fn new(bind: std::net::SocketAddr) -> std::io::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(bind).await?;

        Ok(Self {
            sock: socket
        })
    }

    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.sock.local_addr()
    }

    pub fn has_pacing(&self) -> bool {
        false
    }

    pub async fn send_dgram(&self, buf: &[u8], send_info: &quiche::SendInfo) -> std::io::Result<usize> {
        self.sock.send_to(buf, send_info.to).await
    }

    pub async fn recv_dgram(&self, buf: &mut [u8]) -> std::io::Result<(usize, quiche::RecvInfo)> {
        let (len, from) = self.sock.recv_from(buf).await?;

        Ok((len, quiche::RecvInfo {
            from,
            to: self.local_addr()?,
        }))
    }
}