use super::stream;
use rand::prelude::*;
use std::ops::Deref;

#[derive(Clone)]
pub enum ConnectionError {
    Quic(quiche::Error),
    Io(std::io::ErrorKind),
    Connection(quiche::ConnectionError),
}

impl ConnectionError {
    pub fn to_id(&self) -> u64 {
        match self {
            Self::Quic(_) => 0,
            Self::Io(_) => 0,
            Self::Connection(c) => c.error_code,
        }
    }
}

impl std::fmt::Debug for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quic(q) => f.write_fmt(format_args!("QUIC({:?})", q)),
            Self::Io(e) => f.write_fmt(format_args!("IO({:?})", e)),
            Self::Connection(e) => f.write_fmt(format_args!(
                "Connection(is_app={}, error_code={:x}, reason={})",
                e.is_app,
                e.error_code,
                String::from_utf8_lossy(&e.reason)
            )),
        }
    }
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl std::error::Error for ConnectionError {}

type ConnectionResult<T> = Result<T, ConnectionError>;

impl From<quiche::Error> for ConnectionError {
    fn from(value: quiche::Error) -> Self {
        Self::Quic(value)
    }
}

impl From<quiche::ConnectionError> for ConnectionError {
    fn from(value: quiche::ConnectionError) -> Self {
        Self::Connection(value)
    }
}

impl From<std::io::Error> for ConnectionError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.kind())
    }
}

impl From<std::io::ErrorKind> for ConnectionError {
    fn from(value: std::io::ErrorKind) -> Self {
        Self::Io(value)
    }
}

impl From<ConnectionError> for std::io::Error {
    fn from(value: ConnectionError) -> Self {
        match value {
            ConnectionError::Io(k) => std::io::Error::new(k, ""),
            o => std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", o)),
        }
    }
}

pub(super) enum Control {
    ShouldSend,
    SendAckEliciting,
    SetQLog(QLogConfig),
    Close {
        app: bool,
        err: u64,
        reason: Vec<u8>,
    },
    StreamSend {
        stream_id: u64,
        data: Vec<u8>,
        fin: bool,
        resp: tokio::sync::oneshot::Sender<ConnectionResult<usize>>,
    },
    StreamRecv {
        stream_id: u64,
        len: usize,
        resp: tokio::sync::oneshot::Sender<ConnectionResult<(Vec<u8>, bool)>>,
    },
    SendNewToken {
        token: Vec<u8>
    }
}

#[derive(Debug)]
pub struct Connection {
    is_server: bool,
    control_tx: tokio::sync::mpsc::Sender<Control>,
    shared_state: std::sync::Arc<SharedConnectionState>,
    new_stream_rx: Option<tokio::sync::mpsc::Receiver<stream::Stream>>,
    new_token_rx: Option<tokio::sync::mpsc::Receiver<Vec<u8>>>,
}

pub struct QLogConfig {
    pub qlog: crate::qlog::QLog,
    pub title: String,
    pub description: String,
    pub level: quiche::QlogLevel,
}

#[derive(Debug)]
pub(super) struct SharedConnectionState {
    connection_established: std::sync::atomic::AtomicBool,
    connection_established_notify: tokio::sync::Mutex<Vec<std::sync::Arc<tokio::sync::Notify>>>,
    connection_closed: std::sync::atomic::AtomicBool,
    connection_closed_notify: tokio::sync::Mutex<Vec<std::sync::Arc<tokio::sync::Notify>>>,
    application_protocol: tokio::sync::RwLock<Vec<u8>>,
    transport_parameters: tokio::sync::RwLock<Option<quiche::TransportParams>>,
    pub(super) connection_error: tokio::sync::RwLock<Option<ConnectionError>>,
}

struct InnerConnectionState {
    conn: quiche::Connection,
    socket: std::sync::Arc<tokio::net::UdpSocket>,
    packet_rx: tokio::sync::mpsc::Receiver<(Vec<u8>, quiche::RecvInfo)>,
    max_datagram_size: usize,
    control_rx: tokio::sync::mpsc::Receiver<Control>,
    control_tx: tokio::sync::mpsc::Sender<Control>,
    new_stream_tx: tokio::sync::mpsc::Sender<stream::Stream>,
    new_token_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    timeout_running: bool,
}

impl Connection {
    pub async fn accept(
        bind_addr: std::net::SocketAddr,
        mut config: quiche::Config,
    ) -> ConnectionResult<NewConnections> {
        let socket = std::sync::Arc::new(
            tokio::net::UdpSocket::bind(bind_addr).await?
        );
        let local_addr = socket.local_addr()?;
        debug!("Listening on {}", local_addr);

        let (new_cons_tx, new_cons_rx) = tokio::sync::mpsc::channel(100);

        tokio::task::spawn(async move {
            let mut buf = [0; 65535];
            let mut clients: std::collections::HashMap<
                quiche::ConnectionId<'static>,
                tokio::sync::mpsc::Sender<(Vec<u8>, quiche::RecvInfo)>,
            > = std::collections::HashMap::new();

            loop {
                let (len, from) = match socket.recv_from(&mut buf).await {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Failed to received UDP packet: {}", e);
                        break;
                    }
                };

                let pkt_buf = &mut buf[..len];

                let hdr = match quiche::Header::from_slice(
                    pkt_buf, quiche::MAX_CONN_ID_LEN,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Parsing packet header failed: {:?}", e);
                        continue;
                    },
                };

                let tx = if !clients.contains_key(&hdr.dcid) {
                    if hdr.ty != quiche::Type::Initial {
                        warn!("Packet is not Initial");
                        continue;
                    }

                    if !quiche::version_is_supported(hdr.version) {
                        let vneg_socket = socket.clone();

                        tokio::task::spawn(async move {
                            let mut buf = [0; 65535];
                            let len = quiche::negotiate_version(&hdr.scid, &hdr.dcid, &mut buf)
                                    .unwrap();
                            let out = &buf[..len];

                            if let Err(e) = vneg_socket.send_to(out, from).await {
                                error!("Failed to send packet: {}", e)
                            }
                        });
                        continue;
                    }

                    let mut cid = [0; quiche::MAX_CONN_ID_LEN];
                    thread_rng().fill(&mut cid[..]);
                    let cid = quiche::ConnectionId::from_vec(cid.to_vec());

                    debug!("New connection: dcid={:?} scid={:?}", hdr.dcid, cid);

                    let conn = quiche::accept(
                        &cid,
                        None,
                        local_addr,
                        from,
                        &mut config,
                    ).unwrap();
                    let (packet_tx, packet_rx) = tokio::sync::mpsc::channel(100);
                    let connection = Self::setup_connection(
                        conn, socket.clone(), packet_rx,  None
                    ).await;

                    if let Err(_) = new_cons_tx.send(connection).await {
                        break;
                    }
                    clients.insert(cid, packet_tx.clone());
                    packet_tx
                } else {
                    clients.get(&hdr.dcid).unwrap().to_owned()
                };

                let recv_info = quiche::RecvInfo {
                    to: local_addr,
                    from,
                };

                if let Err(_) = tx.send((pkt_buf.to_vec(), recv_info)).await {
                    clients.remove(&hdr.dcid);
                }
            }
        });

        Ok(NewConnections {
            new_connections: new_cons_rx
        })
    }

    pub async fn connect(
        peer_addr: std::net::SocketAddr,
        mut config: quiche::Config,
        server_name: Option<&str>,
        qlog: Option<QLogConfig>,
    ) -> ConnectionResult<Self> {
        let bind_addr: std::net::SocketAddr = match peer_addr {
            std::net::SocketAddr::V4(_) => "0.0.0.0:0",
            std::net::SocketAddr::V6(_) => "[::]:0",
        }
        .parse()
        .unwrap();

        let mut cid = [0; quiche::MAX_CONN_ID_LEN];
        thread_rng().fill(&mut cid[..]);
        let cid = quiche::ConnectionId::from_ref(&cid);

        let socket = tokio::net::UdpSocket::bind(bind_addr).await?;
        let local_addr = socket.local_addr()?;
        debug!("Connecting to {} from {}", peer_addr, local_addr);

        let conn = quiche::connect(server_name, &cid, local_addr, peer_addr, &mut config)?;

        let socket = std::sync::Arc::new(socket);
        let (packet_tx, packet_rx) = tokio::sync::mpsc::channel(100);

        let recv_socket = socket.clone();
        tokio::task::spawn(async move {
            let mut buf = [0; 65535];
            loop {
                let (len, addr) = match recv_socket.recv_from(&mut buf).await {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Failed to read UDP packet: {}", e);
                        break;
                    }
                };
                let recv_info = quiche::RecvInfo {
                    from: addr,
                    to: local_addr
                };

                if let Err(_) = packet_tx.send((buf[..len].to_vec(), recv_info)).await {
                    break
                }
            }
        });

        Ok(Self::setup_connection(
            conn, socket, packet_rx, qlog
        ).await)
    }

    async fn setup_connection(
        mut conn: quiche::Connection,
        socket: std::sync::Arc<tokio::net::UdpSocket>,
        packet_rx: tokio::sync::mpsc::Receiver<(Vec<u8>, quiche::RecvInfo)>,
        qlog: Option<QLogConfig>,
    ) -> Self {
        if let Some(qlog) = qlog {
            conn.set_qlog_with_level(
                Box::new(qlog.qlog),
                qlog.title,
                qlog.description,
                qlog.level,
            );
        }
        let max_datagram_size = conn.max_send_udp_payload_size();

        let (control_tx, control_rx) = tokio::sync::mpsc::channel(25);
        let (new_stream_tx, new_stream_rx) = tokio::sync::mpsc::channel(25);
        let (new_token_tx, new_token_rx) = tokio::sync::mpsc::channel(25);

        let shared_connection_state = std::sync::Arc::new(SharedConnectionState {
            connection_established: std::sync::atomic::AtomicBool::new(false),
            connection_established_notify: tokio::sync::Mutex::new(Vec::new()),
            connection_closed: std::sync::atomic::AtomicBool::new(false),
            connection_closed_notify: tokio::sync::Mutex::new(Vec::new()),
            connection_error: tokio::sync::RwLock::new(None),
            application_protocol: tokio::sync::RwLock::new(Vec::new()),
            transport_parameters: tokio::sync::RwLock::new(None),
        });

        let connection = Connection {
            is_server: conn.is_server(),
            control_tx: control_tx.clone(),
            shared_state: shared_connection_state.clone(),
            new_stream_rx: Some(new_stream_rx),
            new_token_rx: Some(new_token_rx),
        };

        shared_connection_state.run(InnerConnectionState {
            conn,
            socket,
            packet_rx,
            max_datagram_size,
            control_rx,
            control_tx,
            new_stream_tx,
            new_token_tx,
            timeout_running: false,
        });

        connection.should_send().await.unwrap();

        connection
    }

    async fn send_control(&self, control: Control) -> ConnectionResult<()> {
        if let Some(err) = self
            .shared_state
            .connection_error
            .read()
            .await
            .deref()
            .clone()
        {
            return Err(err);
        }
        match self.control_tx.try_send(control) {
            Ok(_) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                if let Some(err) = self
                    .shared_state
                    .connection_error
                    .read()
                    .await
                    .deref()
                    .clone()
                {
                    return Err(err);
                }
                return Err(std::io::ErrorKind::ConnectionReset.into());
            }
        }
        Ok(())
    }

    async fn should_send(&self) -> ConnectionResult<()> {
        self.send_control(Control::ShouldSend).await
    }

    pub async fn established(&self) -> ConnectionResult<()> {
        if let Some(err) = self
            .shared_state
            .connection_error
            .read()
            .await
            .deref()
            .clone()
        {
            return Err(err);
        }
        if self
            .shared_state
            .connection_established
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }
        let notify = std::sync::Arc::new(tokio::sync::Notify::new());
        self.shared_state
            .connection_established_notify
            .lock()
            .await
            .push(notify.clone());
        if let Some(err) = self
            .shared_state
            .connection_error
            .read()
            .await
            .deref()
            .clone()
        {
            return Err(err);
        }
        notify.notified().await;
        if let Some(err) = self
            .shared_state
            .connection_error
            .read()
            .await
            .deref()
            .clone()
        {
            return Err(err);
        }
        Ok(())
    }

    pub async fn application_protocol(&self) -> Vec<u8> {
        self.shared_state.application_protocol.read().await.clone()
    }

    pub async fn transport_parameters(&self) -> Option<quiche::TransportParams> {
        self.shared_state.transport_parameters.read().await.clone()
    }

    pub async fn set_qlog(&self, qlog: QLogConfig) -> ConnectionResult<()> {
        self.send_control(Control::SetQLog(qlog)).await
    }

    pub async fn send_ack_eliciting(&self) -> ConnectionResult<()> {
        self.send_control(Control::SendAckEliciting).await
    }

    pub async fn send_new_token(&self, token: Vec<u8>) -> ConnectionResult<()> {
        self.send_control(Control::SendNewToken {
            token
        }).await
    }

    pub async fn close(&self, app: bool, err: u64, reason: Vec<u8>) -> ConnectionResult<()> {
        let notify = std::sync::Arc::new(tokio::sync::Notify::new());
        self.shared_state
            .connection_established_notify
            .lock()
            .await
            .push(notify.clone());
        self.send_control(Control::Close { app, err, reason })
            .await?;
        notify.notified().await;
        if let Some(err) = self
            .shared_state
            .connection_error
            .read()
            .await
            .deref()
            .clone()
        {
            return Err(err);
        }
        Ok(())
    }

    pub fn is_server(&self) -> bool {
        self.is_server
    }

    pub async fn new_stream(&self, stream_id: u64, bidi: bool) -> ConnectionResult<stream::Stream> {
        Ok(stream::Stream::new(
            self.is_server,
            stream::StreamID::new(stream_id, bidi, self.is_server),
            self.shared_state.clone(),
            self.control_tx.clone(),
        ))
    }

    pub async fn next_peer_stream(&mut self) -> ConnectionResult<Option<stream::Stream>> {
        match self.new_stream_rx.as_mut().unwrap().recv().await {
            Some(s) => Ok(Some(s)),
            None => {
                let err = self.shared_state.connection_error.read().await.clone();
                match err {
                    None => Ok(None),
                    Some(e) => Err(e)
                }
            }
        }
    }

    pub fn peer_streams(&mut self) -> ConnectionRecv<stream::Stream> {
        ConnectionRecv {
            rx: self.new_stream_rx.take().unwrap(),
            shared_state: self.shared_state.clone(),
        }
    }

    pub async fn next_new_token(&mut self) -> ConnectionResult<Option<Vec<u8>>> {
        match self.new_token_rx.as_mut().unwrap().recv().await {
            Some(s) => Ok(Some(s)),
            None => {
                let err = self.shared_state.connection_error.read().await.clone();
                match err {
                    None => Ok(None),
                    Some(e) => Err(e)
                }
            }
        }
    }

    pub fn new_tokens(&mut self) -> ConnectionRecv<Vec<u8>> {
        ConnectionRecv {
            rx: self.new_token_rx.take().unwrap(),
            shared_state: self.shared_state.clone(),
        }
    }
}

#[derive(Debug)]
pub struct ConnectionRecv<T> {
    rx: tokio::sync::mpsc::Receiver<T>,
    shared_state: std::sync::Arc<SharedConnectionState>,
}

impl<T> ConnectionRecv<T> {
    pub async fn next(&mut self) -> ConnectionResult<Option<T>> {
        match self.rx.recv().await {
            Some(s) => Ok(Some(s)),
            None => {
                let err = self.shared_state.connection_error.read().await.clone();
                match err {
                    None => Ok(None),
                    Some(e) => Err(e)
                }
            },
        }
    }
}

#[derive(Debug)]
pub struct NewConnections {
    new_connections: tokio::sync::mpsc::Receiver<Connection>
}

impl NewConnections {
    pub async fn next(&mut self) -> Option<Connection> {
        self.new_connections.recv().await
    }
}


struct PendingReceive {
    stream_id: u64,
    read_len: usize,
    resp: tokio::sync::oneshot::Sender<ConnectionResult<(Vec<u8>, bool)>>,
}

impl SharedConnectionState {
    fn run(self: std::sync::Arc<Self>, mut inner: InnerConnectionState) {
        let (timeout_tx, mut timeout_rx) = tokio::sync::mpsc::channel(1);

        tokio::task::spawn(async move {
            let mut out = vec![0; inner.max_datagram_size];
            let mut pending_recv: Vec<PendingReceive> = vec![];
            let mut known_stream_ids = std::collections::HashSet::new();

            'outer: loop {
                tokio::select! {
                    res = inner.packet_rx.recv() => {
                        let (mut pkt, recv_info) = match res {
                            Some(v) => v,
                            None => {
                                self.set_error(std::io::ErrorKind::ConnectionReset.into()).await;
                                break;
                            }
                        };

                        let read = match inner.conn.recv(&mut pkt, recv_info) {
                            Ok(v) => v,
                            Err(quiche::Error::Done) => {
                                continue;
                            },
                            Err(e) => {
                                self.set_error(e.into()).await;
                                break;
                            },
                        };
                        trace!("Received {} bytes", read);
                        if inner.conn.is_established() {
                            self.set_established(inner.conn.application_proto(), inner.conn.peer_transport_params()).await;
                        }
                        inner.control_tx.send(Control::ShouldSend).await.unwrap();

                        let readable = pending_recv
                            .extract_if(|s| inner.conn.stream_readable(s.stream_id))
                            .collect::<Vec<_>>();
                        for s in readable {
                            let mut buf = vec![0u8; s.read_len];
                            match inner.conn.stream_recv(s.stream_id, &mut buf) {
                                Ok((read, fin)) => {
                                    let out = buf[..read].to_vec();
                                    let _ = s.resp.send(Ok((out, fin)));
                                }
                                Err(e) => {
                                    let _ = s.resp.send(Err(e.into()));
                                }
                            }
                        }

                        let new_stream_ids = inner.conn.readable().filter(|stream_id| {
                            let client_flag = stream_id & 1;
                            if inner.conn.is_server() && client_flag == 1 {
                                return false;
                            }
                            if !inner.conn.is_server() && client_flag == 0 {
                                return false;
                            }
                            if known_stream_ids.contains(stream_id) {
                                return false;
                            }
                            known_stream_ids.insert(*stream_id);
                            true
                        }).collect::<Vec<_>>();
                        for stream in new_stream_ids {
                            let _ = inner.new_stream_tx.send(stream::Stream::new(
                                inner.conn.is_server(), stream::StreamID(stream),
                                self.clone(), inner.control_tx.clone(),
                            )).await;
                        }

                        while let Some(token) = inner.conn.recv_new_token() {
                           let _ = inner.new_token_tx.try_send(token);
                        }

                        trace!("Receive done");
                    }
                    c = inner.control_rx.recv() => {
                        let c = match c {
                            Some(c) => c,
                            None => break
                        };
                        match c {
                            Control::ShouldSend => {
                                let mut packets = vec![];
                                loop {
                                    let (write, send_info) = match inner.conn.send(&mut out) {
                                        Ok(v) => v,
                                        Err(quiche::Error::Done) => {
                                            break;
                                        },
                                        Err(e) => {
                                            self.set_error(e.into()).await;
                                            break 'outer;
                                        }
                                    };
                                    packets.push((send_info, (&out[..write]).to_vec()));
                                    if let Some(timeout) = inner.conn.timeout() {
                                        if !inner.timeout_running {
                                            inner.timeout_running = true;
                                            let inner_timeout_tx = timeout_tx.clone();
                                            tokio::task::spawn(async move {
                                                tokio::time::sleep(timeout).await;
                                                let _ = inner_timeout_tx.send(()).await;
                                            });
                                        }
                                    }
                                    if inner.conn.is_established() {
                                        self.set_established(inner.conn.application_proto(), inner.conn.peer_transport_params()).await;
                                    }
                                }
                                for (send_info, packet) in &packets {
                                    if let Err(e) = inner.socket.send_to(packet, &send_info.to).await {
                                        self.set_error(e.into()).await;
                                        break;
                                    }
                                    trace!("Sent {} bytes", packet.len());
                                }
                            },
                            Control::SendAckEliciting => {
                                if let Err(e) = inner.conn.send_ack_eliciting() {
                                    self.set_error(e.into()).await;
                                    break;
                                }
                                inner.control_tx.send(Control::ShouldSend).await.unwrap();
                            }
                            Control::StreamSend { stream_id, data, fin, resp} => {
                                let _ = resp.send(
                                    inner.conn.stream_send(stream_id, &data, fin)
                                        .map_err(|e| e.into())
                                );
                                inner.control_tx.send(Control::ShouldSend).await.unwrap();
                            }
                            Control::StreamRecv { stream_id, len, resp } => {
                                let mut buf = vec![0u8; len];
                                match inner.conn.stream_recv(stream_id, &mut buf) {
                                    Ok((read, fin)) => {
                                        let out = buf[..read].to_vec();
                                        let _ = resp.send(Ok((out, fin)));
                                    }
                                    Err(quiche::Error::Done) => {
                                        pending_recv.push(PendingReceive {
                                            stream_id,
                                            read_len: len,
                                            resp
                                        });
                                    }
                                    Err(e) => {
                                        let _ = resp.send(Err(e.into()));
                                    }
                                }
                                inner.control_tx.send(Control::ShouldSend).await.unwrap();
                            }
                            // Control::StreamShutdown { stream_id, direction, err, resp} => {
                            //     let _ = resp.send(
                            //         inner.conn.stream_shutdown(stream_id, direction, err)
                            //             .map_err(|e| e.into())
                            //     );
                            // }
                            Control::SetQLog(qlog) => {
                                inner.conn.set_qlog_with_level(
                                    Box::new(qlog.qlog),
                                    qlog.title,
                                    qlog.description,
                                    qlog.level,
                                );
                            }
                            Control::SendNewToken {
                                token
                            } => {
                                inner.conn.send_new_token(&token);
                                inner.control_tx.send(Control::ShouldSend).await.unwrap();
                            }
                            Control::Close { app, err, reason } => {
                                if let Err(e) = inner.conn.close(app, err, &reason) {
                                    self.set_error(e.into()).await;
                                    break;
                                }
                                inner.control_tx.send(Control::ShouldSend).await.unwrap();
                            }
                        }
                    }
                    _ = timeout_rx.recv() => {
                        trace!("On timeout");
                        inner.timeout_running = false;
                        inner.conn.on_timeout();
                        inner.control_tx.send(Control::ShouldSend).await.unwrap();
                    }
                }

                if inner.conn.is_closed() {
                    if let Some(err) = inner.conn.peer_error() {
                        self.connection_error
                            .write()
                            .await
                            .replace(err.clone().into());
                    } else if let Some(err) = inner.conn.local_error() {
                        self.connection_error
                            .write()
                            .await
                            .replace(err.clone().into());
                    } else if inner.conn.is_timed_out() {
                        self.connection_error
                            .write()
                            .await
                            .replace(std::io::ErrorKind::TimedOut.into());
                    } else {
                        self.connection_error
                            .write()
                            .await
                            .replace(std::io::ErrorKind::ConnectionReset.into());
                    }
                    self.set_closed().await;
                    break;
                }
            }
            trace!("Connection closed");
        });
    }

    async fn set_error(&self, error: ConnectionError) {
        self.connection_error.write().await.replace(error);
        self.notify_connection_established().await;
    }

    async fn notify_connection_established(&self) {
        for n in self.connection_established_notify.lock().await.drain(..) {
            n.notify_one();
        }
    }

    async fn notify_connection_closed(&self) {
        for n in self.connection_closed_notify.lock().await.drain(..) {
            n.notify_one();
        }
        self.notify_connection_established().await;
    }

    async fn set_established(&self, alpn: &[u8], transport_params: Option<&quiche::TransportParams>) {
        if !self.connection_established.load(std::sync::atomic::Ordering::Relaxed) {
            self.connection_established
                .store(true, std::sync::atomic::Ordering::Relaxed);
            *self.application_protocol.write().await = alpn.to_vec();
            *self.transport_parameters.write().await = transport_params.map(|p| p.to_owned());
            self.notify_connection_established().await;
        }
    }

    async fn set_closed(&self) {
        self.connection_closed
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.notify_connection_closed().await;
    }
}
