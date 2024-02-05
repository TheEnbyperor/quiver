use super::{stream, socket};
use rand::prelude::*;
use futures::FutureExt;
use std::ops::Deref;
use std::time::Duration;

#[derive(Clone)]
pub enum ConnectionError {
    Quic(quiche::Error),
    Io((std::io::ErrorKind, Option<std::sync::Arc<Box<dyn std::error::Error + Send + Sync>>>)),
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
            Self::Io(e) => f.write_fmt(format_args!("IO({:?}, {:?})", e.0, e.1)),
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
        let is_quic_err = value.get_ref().and_then(|e| {
            e.downcast_ref::<ConnectionError>()
        }).is_some();
        if is_quic_err {
            let err = *value.into_inner().unwrap()
                .downcast::<ConnectionError>().unwrap();
            err
        } else {
            Self::Io((value.kind(), value.into_inner().map(std::sync::Arc::new)))
        }
    }
}

impl From<std::io::ErrorKind> for ConnectionError {
    fn from(value: std::io::ErrorKind) -> Self {
        Self::Io((value, None))
    }
}

impl From<ConnectionError> for std::io::Error {
    fn from(value: ConnectionError) -> Self {
        match value {
            o => std::io::Error::new(std::io::ErrorKind::Other, o),
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
    },
    SetupCarefulResume {
        previous_rtt: Duration,
        previous_cwnd: usize,
        resp: tokio::sync::oneshot::Sender<ConnectionResult<()>>,
    }
}

#[derive(Debug)]
pub struct Connection {
    new_stream_rx: Option<tokio::sync::mpsc::Receiver<stream::Stream>>,
    new_token_rx: Option<tokio::sync::mpsc::Receiver<Vec<u8>>>,
    new_cr_event_rx: Option<tokio::sync::watch::Receiver<Option<quiche::CREvent>>>,
    send_half: ConnectionSendHalf
}

#[derive(Debug, Clone)]
pub struct ConnectionSendHalf {
    is_server: bool,
    scid: quiche::ConnectionId<'static>,
    local_addr: std::net::SocketAddr,
    peer_addr: std::net::SocketAddr,
    control_tx: tokio::sync::mpsc::UnboundedSender<Control>,
    shared_state: std::sync::Arc<SharedConnectionState>,
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
    dcid: tokio::sync::RwLock<Option<quiche::ConnectionId<'static>>>,
    peer_token: tokio::sync::RwLock<Option<Vec<u8>>>,
    transport_parameters: tokio::sync::RwLock<Option<quiche::TransportParams>>,
    pub(super) connection_error: tokio::sync::RwLock<Option<ConnectionError>>,
}

struct InnerConnectionState {
    scid: quiche::ConnectionId<'static>,
    conn: quiche::Connection,
    socket: std::sync::Arc<socket::UdpSocket>,
    packet_rx: tokio::sync::mpsc::Receiver<(Vec<u8>, quiche::RecvInfo)>,
    max_datagram_size: usize,
    control_rx: tokio::sync::mpsc::UnboundedReceiver<Control>,
    control_tx: tokio::sync::mpsc::UnboundedSender<Control>,
    new_stream_tx: tokio::sync::mpsc::Sender<stream::Stream>,
    new_token_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    new_cr_event_tx: tokio::sync::watch::Sender<Option<quiche::CREvent>>,
}

impl Connection {
    pub async fn accept(
        bind_addr: std::net::SocketAddr,
        mut config: quiche::Config,
    ) -> ConnectionResult<NewConnections> {
        let socket = std::sync::Arc::new(
            socket::UdpSocket::new(bind_addr).await?
        );

        if !socket.has_pacing() {
            config.enable_pacing(false);
        }

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
                let (len, recv_info) = match socket.recv_dgram(&mut buf).await {
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

                            if let Err(e) = vneg_socket.send_dgram(out, &quiche::SendInfo {
                                from: local_addr,
                                to: recv_info.from,
                                at: std::time::Instant::now()
                            }).await {
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
                        recv_info.from,
                        &mut config,
                    ).unwrap();
                    let (packet_tx, packet_rx) = tokio::sync::mpsc::channel(100);
                    let connection = Self::setup_connection(
                        conn, cid.clone(), recv_info.from, socket.clone(), packet_rx, None, None
                    ).await;

                    if let Err(_) = new_cons_tx.send(connection).await {
                        break;
                    }
                    clients.insert(cid, packet_tx.clone());
                    packet_tx
                } else {
                    clients.get(&hdr.dcid).unwrap().to_owned()
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
        bind_addr: Option<std::net::SocketAddr>,
        token: Option<&[u8]>,
        qlog: Option<QLogConfig>,
    ) -> ConnectionResult<Self> {
        let bind_addr: std::net::SocketAddr = match bind_addr {
            Some(b) => b,
            None => match peer_addr {
                std::net::SocketAddr::V4(_) => std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
                    std::net::Ipv4Addr::UNSPECIFIED, 0,
                )),
                std::net::SocketAddr::V6(_) => std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
                    std::net::Ipv6Addr::UNSPECIFIED, 0, 0, 0
                )),
            }
        };

        let mut cid = [0; quiche::MAX_CONN_ID_LEN];
        thread_rng().fill(&mut cid[..]);
        let scid = quiche::ConnectionId::from_ref(&cid);

        let socket = socket::UdpSocket::new(bind_addr).await?;

        if !socket.has_pacing() {
            config.enable_pacing(false);
        }

        let local_addr = socket.local_addr()?;
        debug!("Connecting to {} from {}", peer_addr, local_addr);

        let conn = quiche::connect(server_name, &scid, local_addr, peer_addr, &mut config)?;

        let socket = std::sync::Arc::new(socket);
        let (packet_tx, packet_rx) = tokio::sync::mpsc::channel(100);

        let recv_socket = socket.clone();
        tokio::task::spawn(async move {
            let mut buf = [0; 65535];
            loop {
                let (len, recv_info) = match recv_socket.recv_dgram(&mut buf).await {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Failed to read UDP packet: {}", e);
                        break;
                    }
                };

                if let Err(_) = packet_tx.send((buf[..len].to_vec(), recv_info)).await {
                    break
                }
            }
        });

        Ok(Self::setup_connection(
            conn, scid, peer_addr, socket, packet_rx, token, qlog
        ).await)
    }

    async fn setup_connection<'a>(
        mut conn: quiche::Connection,
        scid: quiche::ConnectionId<'a>,
        peer_addr: std::net::SocketAddr,
        socket: std::sync::Arc<socket::UdpSocket>,
        packet_rx: tokio::sync::mpsc::Receiver<(Vec<u8>, quiche::RecvInfo)>,
        token: Option<&[u8]>,
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

        if let Some(token) = token {
            conn.set_token(token);
        }

        let max_datagram_size = conn.max_send_udp_payload_size();

        let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
        let (new_stream_tx, new_stream_rx) = tokio::sync::mpsc::channel(25);
        let (new_token_tx, new_token_rx) = tokio::sync::mpsc::channel(25);
        let (new_cr_event_tx, new_cr_event_rx) = tokio::sync::watch::channel(None);

        let shared_connection_state = std::sync::Arc::new(SharedConnectionState {
            connection_established: std::sync::atomic::AtomicBool::new(false),
            connection_established_notify: tokio::sync::Mutex::new(Vec::new()),
            connection_closed: std::sync::atomic::AtomicBool::new(false),
            connection_closed_notify: tokio::sync::Mutex::new(Vec::new()),
            connection_error: tokio::sync::RwLock::new(None),
            application_protocol: tokio::sync::RwLock::new(Vec::new()),
            dcid: tokio::sync::RwLock::new(None),
            peer_token: tokio::sync::RwLock::new(None),
            transport_parameters: tokio::sync::RwLock::new(None),
        });

        let scid = scid.into_owned();

        let connection = Connection {
            new_stream_rx: Some(new_stream_rx),
            new_token_rx: Some(new_token_rx),
            new_cr_event_rx: Some(new_cr_event_rx),
            send_half: ConnectionSendHalf {
                scid: scid.clone(),
                is_server: conn.is_server(),
                local_addr: socket.local_addr().unwrap(),
                peer_addr,
                control_tx: control_tx.clone(),
                shared_state: shared_connection_state.clone(),
            }
        };

        shared_connection_state.run(InnerConnectionState {
            conn,
            scid,
            socket,
            packet_rx,
            max_datagram_size,
            control_rx,
            control_tx,
            new_stream_tx,
            new_token_tx,
            new_cr_event_tx,
        });

        connection.send_half.should_send().await.unwrap();

        connection
    }

    pub async fn next_peer_stream(&mut self) -> ConnectionResult<Option<stream::Stream>> {
        match self.new_stream_rx.as_mut().unwrap().recv().await {
            Some(s) => Ok(Some(s)),
            None => {
                let err = self.send_half.shared_state.connection_error.read().await.clone();
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
            shared_state: self.send_half.shared_state.clone(),
        }
    }

    pub async fn next_new_token(&mut self) -> ConnectionResult<Option<Vec<u8>>> {
        match self.new_token_rx.as_mut().unwrap().recv().await {
            Some(s) => Ok(Some(s)),
            None => {
                let err = self.send_half.shared_state.connection_error.read().await.clone();
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
            shared_state: self.send_half.shared_state.clone(),
        }
    }

    pub async fn next_cr_event(&mut self) -> Option<quiche::CREvent> {
        if let Err(_) = self.new_cr_event_rx.as_mut().unwrap().changed().await {
            return None;
        }
        *self.new_cr_event_rx.as_mut().unwrap().borrow_and_update()
    }

    pub fn cr_events(&mut self) -> NewCREvents {
        NewCREvents {
            new_cr_events: self.new_cr_event_rx.take().unwrap()
        }
    }

    pub fn send_half(&self) -> ConnectionSendHalf {
        self.send_half.clone()
    }

    pub async fn established(&self) -> ConnectionResult<()> {
        self.send_half.established().await
    }

    pub async fn application_protocol(&self) -> Vec<u8> {
        self.send_half.application_protocol().await
    }

    pub async fn transport_parameters(&self) -> Option<quiche::TransportParams> {
        self.send_half.transport_parameters().await
    }

    pub async fn peer_token(&self) -> Option<Vec<u8>> {
        self.send_half.peer_token().await
    }

    pub async fn set_qlog(&self, qlog: QLogConfig) -> ConnectionResult<()> {
        self.send_half.set_qlog(qlog).await
    }

    pub async fn send_ack_eliciting(&self) -> ConnectionResult<()> {
        self.send_half.send_ack_eliciting().await
    }

    pub async fn send_new_token(&self, token: Vec<u8>) -> ConnectionResult<()> {
        self.send_half.send_new_token(token).await
    }

    pub async fn close(&self, app: bool, err: u64, reason: Vec<u8>) -> ConnectionResult<()> {
        self.send_half.close(app, err, reason).await
    }

    pub fn is_server(&self) -> bool {
        self.send_half.is_server()
    }

    pub fn scid<'a>(&'a self) -> &quiche::ConnectionId<'a> {
        self.send_half.scid()
    }

    pub async fn dcid(&self) -> Option<quiche::ConnectionId<'static>> {
        self.send_half.dcid().await
    }

    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.send_half.local_addr()
    }

    pub fn peer_addr(&self) -> std::net::SocketAddr {
        self.send_half.peer_addr()
    }

    pub async fn new_stream(&self, stream_id: u64, bidi: bool) -> ConnectionResult<stream::Stream> {
        self.send_half.new_stream(stream_id, bidi).await
    }

    pub async fn setup_careful_resume(&self, previous_rtt: Duration, previous_cwnd: usize) -> ConnectionResult<()> {
        self.send_half.setup_careful_resume(previous_rtt, previous_cwnd).await
    }
}

impl ConnectionSendHalf {
    async fn make_error(&self) -> ConnectionError {
        if let Some(err) = self
            .shared_state
            .connection_error
            .read()
            .await
            .deref()
            .clone()
        {
            return err;
        }
        std::io::ErrorKind::ConnectionReset.into()
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
        match self.control_tx.send(control) {
            Ok(_) => {}
            Err(_) => {
                return Err(self.make_error().await);
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

    pub async fn peer_token(&self) -> Option<Vec<u8>> {
        self.shared_state.peer_token.read().await.clone()
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

    pub fn scid<'a>(&'a self) -> &quiche::ConnectionId<'a> {
        &self.scid
    }
    pub async fn dcid(&self) -> Option<quiche::ConnectionId<'static>> {
        self.shared_state.dcid.read().await.clone()
    }

    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.local_addr
    }

    pub fn peer_addr(&self) -> std::net::SocketAddr {
        self.peer_addr
    }

    pub async fn new_stream(&self, stream_id: u64, bidi: bool) -> ConnectionResult<stream::Stream> {
        Ok(stream::Stream::new(
            self.is_server,
            stream::StreamID::new(stream_id, bidi, self.is_server),
            self.shared_state.clone(),
            self.control_tx.clone(),
        ))
    }

    pub async fn setup_careful_resume(&self, previous_rtt: Duration, previous_cwnd: usize) -> ConnectionResult<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send_control(Control::SetupCarefulResume {
            previous_rtt,
            previous_cwnd,
            resp: tx
        }).await?;
        match rx.await {
            Ok(r) => r,
            Err(_) => Err(self.make_error().await)
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
pub struct NewCREvents {
    new_cr_events: tokio::sync::watch::Receiver<Option<quiche::CREvent>>
}

impl NewCREvents {
    pub async fn next(&mut self) -> Option<quiche::CREvent> {
        if let Err(_) = self.new_cr_events.changed().await {
            return None;
        }
        *self.new_cr_events.borrow_and_update()
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

struct PendingSend {
    stream_id: u64,
    data: Vec<u8>,
    fin: bool,
    resp: tokio::sync::oneshot::Sender<ConnectionResult<usize>>,
}

impl SharedConnectionState {
    fn run(self: std::sync::Arc<Self>, mut inner: InnerConnectionState) {
        tokio::task::spawn(async move {
            let mut out = vec![0; inner.max_datagram_size];
            let mut pending_recv: Vec<PendingReceive> = vec![];
            let mut pending_send: Vec<PendingSend> = vec![];
            let mut known_stream_ids = std::collections::HashSet::new();

            'outer: loop {
                let timeout = match inner.conn.timeout() {
                    Some(d) => tokio::time::sleep(d).boxed(),
                    None => futures::future::pending().boxed()
                };
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
                        trace!("{:?} Received {} bytes", inner.scid, read);
                        if inner.conn.is_established() {
                            self.set_established(
                                inner.conn.application_proto(),
                                inner.conn.peer_transport_params(),
                                inner.conn.peer_token(),
                                inner.conn.destination_id().into_owned()
                            ).await;
                        }

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

                        inner.control_tx.send(Control::ShouldSend).unwrap();

                        trace!("{:?} Receive done", inner.scid);
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
                                }
                                for (send_info, packet) in &packets {
                                    if let Err(e) = inner.socket.send_dgram(packet, send_info).await {
                                        self.set_error(e.into()).await;
                                        break;
                                    }
                                    trace!("{:?} Sent {} bytes", inner.scid, packet.len());
                                }

                                if inner.conn.is_established() {
                                    self.set_established(
                                        inner.conn.application_proto(),
                                        inner.conn.peer_transport_params(),
                                        inner.conn.peer_token(),
                                        inner.conn.destination_id().into_owned()
                                    ).await;
                                }

                                let writable = pending_send
                                    .extract_if(|s| inner.conn.stream_capacity(s.stream_id)
                                        .map(|c| c > 0).unwrap_or_default()
                                    )
                                    .collect::<Vec<_>>();
                                for s in writable {
                                    inner.control_tx.send(Control::StreamSend {
                                        stream_id: s.stream_id,
                                        data: s.data,
                                        fin: s.fin,
                                        resp: s.resp
                                    }).unwrap();
                                }
                            },
                            Control::SendAckEliciting => {
                                if let Err(e) = inner.conn.send_ack_eliciting() {
                                    self.set_error(e.into()).await;
                                    break;
                                }
                                inner.control_tx.send(Control::ShouldSend).unwrap();
                            }
                            Control::StreamSend { stream_id, data, fin, resp} => {
                                match inner.conn.stream_send(stream_id, &data, fin) {
                                    Ok(s) => {
                                        let _ = resp.send(Ok(s));
                                    },
                                    Err(quiche::Error::Done) => {
                                        pending_send.push(PendingSend {
                                            stream_id,
                                            data,
                                            fin,
                                            resp
                                        });
                                    }
                                    Err(e) => {
                                        let _ = resp.send(Err(e.into()));
                                    }
                                }
                                inner.control_tx.send(Control::ShouldSend).unwrap();
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
                                inner.control_tx.send(Control::ShouldSend).unwrap();
                            }
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
                                inner.control_tx.send(Control::ShouldSend).unwrap();
                            }
                            Control::SetupCarefulResume {
                                previous_rtt, previous_cwnd, resp
                            } => {
                                let _ = resp.send(
                                    inner.conn.setup_careful_resume(previous_rtt, previous_cwnd)
                                        .map_err(|e| e.into())
                                );
                            }
                            Control::Close { app, err, reason } => {
                                if let Err(e) = inner.conn.close(app, err, &reason) {
                                    if e != quiche::Error::Done {
                                        self.set_error(e.into()).await;
                                    }
                                    break;
                                }
                                inner.control_tx.send(Control::ShouldSend).unwrap();
                            }
                        }
                    }
                    _ = timeout => {
                        trace!("{:?} On timeout", inner.scid);
                        inner.conn.on_timeout();
                        inner.control_tx.send(Control::ShouldSend).unwrap();
                    }
                }

                if let Some(cr_event) = inner.conn.cr_event_next() {
                    inner.new_cr_event_tx.send_replace(Some(cr_event));
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
            trace!("{:?} Connection closed", inner.scid);
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

    async fn set_established(
        &self, alpn: &[u8], transport_params: Option<&quiche::TransportParams>, peer_token: Option<&[u8]>,
        dcid: quiche::ConnectionId<'static>
    ) {
        if !self.connection_established.load(std::sync::atomic::Ordering::Relaxed) {
            self.connection_established
                .store(true, std::sync::atomic::Ordering::Relaxed);
            *self.application_protocol.write().await = alpn.to_vec();
            *self.dcid.write().await = Some(dcid);
            *self.peer_token.write().await = peer_token.map(|t| t.to_vec());
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
