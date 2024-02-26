use super::{error, frames, settings, qpack, qpack::PDU};
use tokio::io::AsyncWriteExt;

const MAX_QPACK_TABLE_CAPACITY: u64 = 65535;

#[derive(Debug)]
enum UniStreamType {
    Control,
    Push,
    QPackEncoder,
    QPackDecoder,
    Other(u64),
}

impl UniStreamType {
    fn to_type_id(&self) -> u64 {
        match self {
            Self::Control => 0x00,
            Self::Push => 0x01,
            Self::QPackEncoder => 0x02,
            Self::QPackDecoder => 0x03,
            Self::Other(i) => *i,
        }
    }

    fn from_type_id(type_id: u64) -> Self {
        match type_id {
            0x00 => Self::Control,
            0x01 => Self::Push,
            0x02 => Self::QPackEncoder,
            0x03 => Self::QPackDecoder,
            i => Self::Other(i),
        }
    }
}

#[derive(Debug)]
pub struct Connection {
    is_server: bool,
    next_bidi_stream_id: u64,
    next_uni_stream_id: u64,
    settings: settings::Settings,
    peer_settings: Option<settings::Settings>,
    control_stream: Option<quiche_tokio::Stream>,
    new_peer_streams: Option<quiche_tokio::ConnectionRecv<quiche_tokio::Stream>>,
    new_requests: Option<tokio::sync::mpsc::Receiver<error::HttpResult<Message>>>,
    shared_state: std::sync::Arc<SharedConnectionState>,
}

#[derive(Debug)]
struct SharedConnectionState {
    connection: quiche_tokio::Connection,
    should_close: std::sync::atomic::AtomicBool,
    go_away: std::sync::atomic::AtomicBool,
    go_away_stream_id: std::sync::atomic::AtomicU64,
    qpack_encoder_stream: tokio::sync::Mutex<Option<quiche_tokio::Stream>>,
    qpack_decoder_stream: tokio::sync::Mutex<Option<quiche_tokio::Stream>>,
    qpack_encoder: tokio::sync::Mutex<qpack::Encoder>,
    qpack_decoder: tokio::sync::Mutex<qpack::Decoder>,
}

#[derive(Default)]
struct PendingPeerStreams {
    control_stream: Option<quiche_tokio::Stream>,
    qpack_encoder_stream: Option<quiche_tokio::Stream>,
    qpack_decoder_stream: Option<quiche_tokio::Stream>,
}

impl Connection {
    pub fn new(mut conn: quiche_tokio::Connection, is_server: bool) -> Self {
        Connection {
            is_server,
            next_bidi_stream_id: 0,
            next_uni_stream_id: 0,
            settings: Default::default(),
            peer_settings: None,
            control_stream: None,
            new_peer_streams: Some(conn.peer_streams()),
            new_requests: None,
            shared_state: std::sync::Arc::new(SharedConnectionState {
                connection: conn,
                should_close: std::sync::atomic::AtomicBool::new(false),
                go_away: std::sync::atomic::AtomicBool::new(false),
                go_away_stream_id: std::sync::atomic::AtomicU64::new(0),
                qpack_encoder_stream: tokio::sync::Mutex::new(None),
                qpack_decoder_stream: tokio::sync::Mutex::new(None),
                qpack_encoder: tokio::sync::Mutex::new(qpack::Encoder::new()),
                qpack_decoder: tokio::sync::Mutex::new(qpack::Decoder::new()),
            }),
        }
    }

    pub async fn setup(&mut self) -> error::HttpResult<()> {
        let res = self.send_settings().await;
        Self::try_result(&self.shared_state, res).await?;

        let res = self.open_uni_stream(UniStreamType::QPackEncoder).await;
        let qpack_encoder_stream = Self::try_result(&self.shared_state, res).await?;
        let res = self.open_uni_stream(UniStreamType::QPackDecoder).await;
        let qpack_decoder_stream = Self::try_result(&self.shared_state, res).await?;
        self.shared_state.qpack_encoder_stream.lock().await.replace(qpack_encoder_stream);
        self.shared_state.qpack_decoder_stream.lock().await.replace(qpack_decoder_stream);

        let mut new_peer_streams = self.new_peer_streams.take().unwrap();
        let (mut new_request_streams_tx, mut new_request_streams_rx) = tokio::sync::mpsc::channel(100);
        let (new_requests_tx, new_requests_rx) = tokio::sync::mpsc::channel(100);
        self.new_requests.replace(new_requests_rx);

        let mut pending_peer_streams = PendingPeerStreams::default();
        while self.peer_settings.is_none()
            || pending_peer_streams.qpack_encoder_stream.is_none()
            || pending_peer_streams.qpack_decoder_stream.is_none()
        {
            let peer_stream = match new_peer_streams.next().await? {
                Some(s) => s,
                None => {
                    return Err(error::Error::MissingSettings.into())
                }
            };
            let res = self
                .handle_new_stream(peer_stream, &mut pending_peer_streams, &mut new_request_streams_tx)
                .await;
            Self::try_result(&self.shared_state, res).await?;
        }

        let peer_control_stream = pending_peer_streams.control_stream.unwrap();
        let peer_qpack_encoder_stream = pending_peer_streams.qpack_encoder_stream.unwrap();
        let peer_qpack_decoder_stream = pending_peer_streams.qpack_decoder_stream.unwrap();

        let control_loop_state = self.shared_state.clone();
        tokio::task::spawn(async move {
            let mut peer_control_stream = tokio::io::BufReader::new(peer_control_stream);
            loop {
                if control_loop_state
                    .should_close
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    break;
                }

                let res = frames::Frame::read(&mut peer_control_stream).await;
                let frame = match Self::try_result(&control_loop_state, res).await {
                    Ok(Some(f)) => f,
                    Ok(None) => {
                        control_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = control_loop_state
                            .connection
                            .close(true, error::Error::ClosedCriticalStream.to_id(), vec![])
                            .await;
                        break;
                    }
                    Err(err) => {
                        warn!("Peer control stream decode error: {}", err);
                        control_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = control_loop_state
                            .connection
                            .close(true, error::Error::ClosedCriticalStream.to_id(), vec![])
                            .await;
                        break;
                    }
                };
                match frame {
                    frames::Frame::Data(_)
                    | frames::Frame::Headers(_)
                    | frames::Frame::PushPromise {
                        field_lines: _,
                        push_id: _,
                    }
                    | frames::Frame::Settings(_) => {
                        control_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = control_loop_state
                            .connection
                            .close(true, error::Error::FrameUnexpected.to_id(), vec![])
                            .await;
                        break;
                    }
                    frames::Frame::GoAway(stream_id) => {
                        control_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        control_loop_state
                            .go_away
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        control_loop_state
                            .go_away_stream_id
                            .store(stream_id, std::sync::atomic::Ordering::Relaxed);
                    }
                    o => {
                        trace!("Received control frame: {:?}", o);
                    }
                }
            }
        });

        let qpack_encoder_loop_state = self.shared_state.clone();
        tokio::task::spawn(async move {
            let mut peer_qpack_encoder_stream =
                tokio::io::BufReader::new(peer_qpack_encoder_stream);
            loop {
                if qpack_encoder_loop_state
                    .should_close
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    break;
                }

                let res = qpack::EncoderInstruction::decode_bytes(
                    &mut peer_qpack_encoder_stream
                ).await;
                let instruction = match Self::try_result(&qpack_encoder_loop_state, res).await {
                    Ok(Some(i)) => i,
                    Ok(None) => {
                        qpack_encoder_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = qpack_encoder_loop_state
                            .connection
                            .close(true, error::Error::ClosedCriticalStream.to_id(), vec![])
                            .await;
                        break;
                    }
                    Err(err) => {
                        warn!("Peer QPACK encoder stream decode error: {}", err);
                        qpack_encoder_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = qpack_encoder_loop_state
                            .connection
                            .close(
                                true,
                                qpack::QPackError::EncoderStreamError.to_id(),
                                vec![],
                            )
                            .await;
                        break;
                    }
                };
                trace!("Encoder instruction: {:?}", instruction);
                let res = qpack_encoder_loop_state
                    .qpack_decoder
                    .lock()
                    .await
                    .handle_encoder_instruction(instruction);
                if Self::try_result(&qpack_encoder_loop_state, res.map_err(Into::into))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let qpack_decoder_loop_state = self.shared_state.clone();
        tokio::task::spawn(async move {
            let mut peer_qpack_decoder_stream =
                tokio::io::BufReader::new(peer_qpack_decoder_stream);
            loop {
                if qpack_decoder_loop_state
                    .should_close
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    break;
                }

                let res = qpack::DecoderInstruction::decode_bytes(
                    &mut peer_qpack_decoder_stream
                ).await;
                let instruction = match Self::try_result(&qpack_decoder_loop_state, res).await {
                    Ok(Some(i)) => i,
                    Ok(None) => {
                        qpack_decoder_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = qpack_decoder_loop_state
                            .connection
                            .close(true, error::Error::ClosedCriticalStream.to_id(), vec![])
                            .await;
                        break;
                    }
                    Err(err) => {
                        warn!("Peer QPACK decoder stream decode error: {}", err);
                        qpack_decoder_loop_state
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = qpack_decoder_loop_state
                            .connection
                            .close(
                                true,
                                qpack::QPackError::DecoderStreamError.to_id(),
                                vec![],
                            )
                            .await;
                        break;
                    }
                };
                trace!("Decoder instruction: {:?}", instruction);
                let res = qpack_decoder_loop_state
                    .qpack_encoder
                    .lock()
                    .await
                    .handle_decoder_instruction(instruction);
                if Self::try_result(&qpack_decoder_loop_state, res.map_err(Into::into))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        if self.is_server {
            tokio::task::spawn(async move {
                loop {
                    let peer_stream = match new_peer_streams.next().await {
                        Ok(Some(s)) => s,
                        Ok(None) => {
                            trace!("Connection closed");
                            break;
                        }
                        Err(e) => {
                            // H3_NO_ERROR
                            if e.to_id() == 0x100 {
                                trace!("Connection closed");
                                break;
                            }
                            warn!("Error receiving requests: {}", e);
                            let _ = new_request_streams_tx.send(Err(e.into())).await;
                            break;
                        }
                    };
                    if !peer_stream.is_bidi() {
                        warn!("Received non-BIDI stream from client after connection setup");
                        let _ = new_request_streams_tx.send(Err(error::Error::GeneralProtocol.into())).await;
                        break;
                    }
                    if new_request_streams_tx.send(Ok(peer_stream)).await.is_err() {
                        break;
                    }
                }
            });
            let requests_state = self.shared_state.clone();
            tokio::task::spawn(async move {
                while let Some(stream) = new_request_streams_rx.recv().await {
                    match stream {
                        Ok(stream) => {
                            let request_tx = new_requests_tx.clone();
                            let request_state = requests_state.clone();
                            tokio::task::spawn(async move {
                                let stream_id = stream.stream_id().full_stream_id();
                                let mut stream = tokio::io::BufStream::new(stream);

                                let request_headers = match Self::get_headers(&request_state, stream_id, &mut stream).await {
                                    Ok(h) => h,
                                    Err(e) => {
                                        warn!("Failed to decode request headers: {}", e);
                                        return;
                                    }
                                };

                                let request = Message {
                                    headers: request_headers,
                                    trailers: None,
                                    stream,
                                    shared_state: request_state
                                };
                                let _ = request_tx.send(Ok(request)).await;
                            });
                        }
                        Err(err) => {
                            let _ = new_requests_tx.send(Err(err)).await;
                        }
                    }
                }
            });
        }

        Ok(())
    }

    pub async fn close(self) -> error::HttpResult<()> {
        self.shared_state
            .should_close
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.shared_state
            .go_away
            .store(true, std::sync::atomic::Ordering::Relaxed);
        match self.shared_state
            .connection
            .close(true, 0x100, vec![])
            .await {
            Ok(()) => Ok(()),
            Err(e) => {
                // H3_NO_ERROR
                if e.to_id() == 0x100 {
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub fn peer_go_away(&self) -> Option<u64> {
        if self
            .shared_state
            .go_away
            .load(std::sync::atomic::Ordering::Acquire)
        {
            Some(
                self.shared_state
                    .go_away_stream_id
                    .load(std::sync::atomic::Ordering::Acquire),
            )
        } else {
            None
        }
    }

    pub async fn send_request(
        &mut self,
        headers: &qpack::Headers<'_>,
    ) -> error::HttpResult<Message> {
        if self
            .shared_state
            .go_away
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(error::Error::FrameUnexpected.into());
        }

        let mut stream = self.open_bidi_stream().await?;
        let stream_id = stream.stream_id().full_stream_id();
        let header_block = self
            .shared_state
            .qpack_encoder
            .lock()
            .await
            .encode_field(stream_id, headers);
        Self::output_qpack_encoder_pending_commands(&self.shared_state).await?;
        let header_block_bytes = header_block.to_vec().await;

        let header_frame = frames::Frame::Headers(header_block_bytes);

        header_frame.write(&mut stream).await?;
        stream.shutdown().await?;

        let mut stream = tokio::io::BufStream::new(stream);
        let response_headers =
            Self::get_headers(&self.shared_state, stream_id, &mut stream).await?;
        Self::output_qpack_deccoder_pending_commands(&self.shared_state).await?;

        Ok(Message {
            headers: response_headers,
            trailers: None,
            stream,
            shared_state: self.shared_state.clone(),
        })
    }

    pub async fn next_request(&mut self) -> error::HttpResult<Option<Message>> {
        let new_requests = match self.new_requests.as_mut() {
            Some(n) => n,
            None => return Err(error::Error::MissingSettings.into())
        };

        new_requests.recv().await.transpose()
    }

    async fn send_settings(&mut self) -> error::HttpResult<()> {
        let mut control_stream = self.open_uni_stream(UniStreamType::Control).await?;

        let settings = frames::Frame::Settings(self.settings.clone());
        settings.write(&mut control_stream).await?;

        self.control_stream = Some(control_stream);
        Ok(())
    }

    async fn output_qpack_encoder_pending_commands(shared_state: &SharedConnectionState) -> error::HttpResult<()> {
        let mut qpack_encoder = shared_state.qpack_encoder.lock().await;
        if qpack_encoder.has_pending_encoder_commands() {
            let commands = qpack_encoder.pending_encoder_commands();
            let mut stream_lock = shared_state.qpack_encoder_stream.lock().await;
            let stream = stream_lock.as_mut().unwrap();
            for command in commands {
                command.encode_bytes(stream).await?;
            }
        }
        Ok(())
    }

    async fn output_qpack_deccoder_pending_commands(shared_state: &SharedConnectionState) -> error::HttpResult<()> {
        let mut qpack_decoder = shared_state.qpack_decoder.lock().await;
        if qpack_decoder.has_pending_decoder_commands() {
            let commands = qpack_decoder.pending_decoder_commands();
            let mut stream_lock = shared_state.qpack_decoder_stream.lock().await;
            let stream = stream_lock.as_mut().unwrap();
            for command in commands {
                command.encode_bytes(stream).await?;
            }
        }
        Ok(())
    }

    async fn handle_new_stream(
        &mut self,
        mut stream: quiche_tokio::Stream,
        pending_peer_streams: &mut PendingPeerStreams,
        new_request_streams_tx: &mut tokio::sync::mpsc::Sender<error::HttpResult<quiche_tokio::Stream>>,
    ) -> error::HttpResult<()> {
        if stream.is_bidi() {
            if self.is_server {
                let _ = new_request_streams_tx.send(Ok(stream)).await;
                return Ok(());
            } else {
                return Err(error::Error::StreamCreation.into());
            }
        }
        let stream_type = quiver_util::vli::read_int_async(&mut stream).await?;
        match UniStreamType::from_type_id(stream_type) {
            UniStreamType::Control => {
                if pending_peer_streams.control_stream.is_some() {
                    return Err(error::Error::StreamCreation.into());
                }
                self.handle_control_stream(&mut stream).await?;
                pending_peer_streams.control_stream = Some(stream);
            }
            UniStreamType::QPackEncoder => {
                if pending_peer_streams.qpack_encoder_stream.is_some() {
                    return Err(error::Error::StreamCreation.into());
                }
                pending_peer_streams.qpack_encoder_stream = Some(stream);
            }
            UniStreamType::QPackDecoder => {
                if pending_peer_streams.qpack_decoder_stream.is_some() {
                    return Err(error::Error::StreamCreation.into());
                }
                pending_peer_streams.qpack_decoder_stream = Some(stream);
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_control_stream(
        &mut self,
        stream: &mut quiche_tokio::Stream,
    ) -> error::HttpResult<()> {
        let res = frames::Frame::read(stream).await;
        let frame = Self::try_result(&self.shared_state, res).await?;
        match frame {
            None => Err(error::HttpError::Transport(
                std::io::ErrorKind::UnexpectedEof.into(),
            )),
            Some(frames::Frame::Settings(settings)) => {
                self.shared_state
                    .qpack_encoder
                    .lock()
                    .await
                    .set_dynamic_table_capacity(std::cmp::min(
                        settings.qpack_max_table_capacity(),
                        MAX_QPACK_TABLE_CAPACITY,
                    ));
                self.peer_settings = Some(settings);
                Ok(())
            }
            _ => Err(error::Error::FrameUnexpected.into()),
        }
    }

    async fn open_bidi_stream(&mut self) -> error::HttpResult<quiche_tokio::Stream> {
        let stream = self
            .shared_state
            .connection
            .new_stream(self.next_bidi_stream_id, true)
            .await?;
        self.next_bidi_stream_id += 1;
        Ok(stream)
    }

    async fn open_uni_stream(
        &mut self,
        stream_type: UniStreamType,
    ) -> error::HttpResult<quiche_tokio::Stream> {
        let mut stream = self
            .shared_state
            .connection
            .new_stream(self.next_uni_stream_id, false)
            .await?;
        quiver_util::vli::write_int_async(&mut stream, stream_type.to_type_id()).await?;
        self.next_uni_stream_id += 1;
        Ok(stream)
    }

    async fn try_result<T>(
        state: &SharedConnectionState,
        result: error::HttpResult<T>,
    ) -> error::HttpResult<T> {
        match result {
            Ok(d) => Ok(d),
            Err(err) => {
                if let error::HttpError::Protocol(proto_err) = &err {
                    state
                        .should_close
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    state
                        .connection
                        .close(true, proto_err.to_id(), vec![])
                        .await?;
                }
                Err(err)
            }
        }
    }

    async fn get_headers<R: tokio::io::AsyncRead + Unpin>(
        state: &SharedConnectionState,
        stream_id: u64,
        stream: &mut R,
    ) -> error::HttpResult<qpack::Headers<'static>> {
        let response_header_bytes = loop {
            let response_frame = match frames::Frame::read(stream).await? {
                Some(f) => f,
                None => {
                    return Err(error::HttpError::Transport(
                        std::io::ErrorKind::UnexpectedEof.into(),
                    ));
                }
            };
            match response_frame {
                frames::Frame::Headers(h) => break h,
                frames::Frame::PushPromise {
                    push_id: _,
                    field_lines: _,
                } => {}
                frames::Frame::Unknown {
                    frame_type: _,
                    data: _,
                } => {}
                _ => {
                    state
                        .should_close
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    state
                        .connection
                        .close(true, error::Error::FrameUnexpected.to_id(), vec![])
                        .await?;
                    return Err(error::Error::FrameUnexpected.into());
                }
            }
        };
        let response_header_block = qpack::FieldLines::from_bytes(&response_header_bytes).await?;

        let mut response_headers =
            Self::decode_headers(state, stream_id, response_header_block.clone()).await?;
        while let qpack::DecodeResult::Wait(notify) = response_headers {
            notify.notified().await;
            response_headers =
                Self::decode_headers(state, stream_id, response_header_block.clone()).await?;
        }
        let response_headers = match response_headers {
            qpack::DecodeResult::Headers(h) => h,
            _ => unreachable!(),
        };

        Ok(response_headers)
    }

    async fn decode_headers(
        state: &SharedConnectionState,
        stream_id: u64,
        field_lines: qpack::FieldLines,
    ) -> error::HttpResult<qpack::DecodeResult> {
        match state
            .qpack_decoder
            .lock()
            .await
            .decode_field(stream_id, field_lines)
        {
            Ok(r) => Ok(r),
            Err(err) => {
                state
                    .should_close
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                state.connection.close(true, err.to_id(), vec![]).await?;
                Err(err.into())
            }
        }
    }

    pub fn inner_connection(&self) -> &quiche_tokio::Connection {
        &self.shared_state.connection
    }
}

pub struct Message {
    headers: qpack::Headers<'static>,
    trailers: Option<qpack::Headers<'static>>,
    stream: tokio::io::BufStream<quiche_tokio::Stream>,
    shared_state: std::sync::Arc<SharedConnectionState>,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("headers", &self.headers)
            .field("trailers", &self.trailers)
            .finish_non_exhaustive()
    }
}

impl Message {
    pub fn headers(&self) -> &qpack::Headers<'static> {
        &self.headers
    }

    pub fn stream_id(&self) -> u64 {
        self.stream.get_ref().stream_id().full_stream_id()
    }

    pub async fn set_max_data(&self, max_data: u64) -> error::HttpResult<()> {
        self.stream.get_ref().set_max_data(max_data).await?;
        Ok(())
    }

    async fn process_trailers(&mut self, trailers_bytes: Vec<u8>) -> error::HttpResult<()> {
        let trailers_block = qpack::FieldLines::from_bytes(&trailers_bytes).await?;
        let mut trailers = Connection::decode_headers(
            &self.shared_state,
            self.stream_id(),
            trailers_block.clone(),
        )
        .await?;
        Connection::output_qpack_deccoder_pending_commands(&self.shared_state).await?;
        while let qpack::DecodeResult::Wait(notify) = trailers {
            notify.notified().await;
            trailers = Connection::decode_headers(
                &self.shared_state,
                self.stream_id(),
                trailers_block.clone(),
            )
            .await?;
            Connection::output_qpack_deccoder_pending_commands(&self.shared_state).await?;
        }
        let trailers = match trailers {
            qpack::DecodeResult::Headers(h) => h,
            _ => unreachable!(),
        };
        self.trailers = Some(trailers);
        Ok(())
    }

    pub async fn get_next_data(&mut self) -> error::HttpResult<Option<Vec<u8>>> {
        loop {
            let response_frame = frames::Frame::read(&mut self.stream).await?;
            match response_frame {
                None => return Ok(None),
                Some(frames::Frame::Data(d)) => {
                    return Ok(Some(d));
                }
                Some(frames::Frame::Headers(h)) => {
                    self.process_trailers(h).await?;
                    return Ok(None);
                }
                Some(frames::Frame::PushPromise {
                    push_id: _,
                    field_lines: _,
                }) => {}
                Some(frames::Frame::Unknown {
                    frame_type: _,
                    data: _,
                }) => {}
                Some(_) => {
                    self.shared_state
                        .should_close
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    self.shared_state
                        .connection
                        .close(true, error::Error::FrameUnexpected.to_id(), vec![])
                        .await?;
                    return Err(error::Error::FrameUnexpected.into());
                }
            }
        }
    }

    pub async fn data(&mut self) -> error::HttpResult<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(data) = self.get_next_data().await? {
            out.extend(data.into_iter());
        }
        Ok(out)
    }

    pub async fn send_headers(&mut self, headers: &qpack::Headers<'_>) -> error::HttpResult<()> {
        let stream_id = self.stream.get_ref().stream_id().full_stream_id();
        let header_block = self
            .shared_state
            .qpack_encoder
            .lock()
            .await
            .encode_field(stream_id, headers);
        Connection::output_qpack_encoder_pending_commands(&self.shared_state).await?;
        let header_block_bytes = header_block.to_vec().await;

        let header_frame = frames::Frame::Headers(header_block_bytes);

        header_frame.write(&mut self.stream).await?;
        Ok(())
    }

    pub async fn send_data<D: Into<Vec<u8>>>(&mut self, data: D) -> error::HttpResult<()> {
        let data_frame = frames::Frame::Data(data.into());
        data_frame.write(&mut self.stream).await?;
        Ok(())
    }

    pub async fn flush(&mut self) -> error::HttpResult<()> {
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn done(&mut self) -> error::HttpResult<()> {
        self.stream.flush().await?;
        self.stream.shutdown().await?;
        Ok(())
    }
}
