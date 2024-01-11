#[derive(Debug)]
pub enum Error {
    GeneralProtocol,
    Internal,
    StreamCreation,
    ClosedCriticalStream,
    FrameUnexpected,
    Frame,
    ExcessiveLoad,
    ID,
    Settings,
    MissingSettings,
    RequestRejected,
    RequestCancelled,
    RequestIncomplete,
    Message,
    Connect,
    VersionFallback,
}

impl Error {
    pub fn to_id(&self) -> u64 {
        match self {
            Self::GeneralProtocol => 0x0101,
            Self::Internal => 0x0102,
            Self::StreamCreation => 0x0103,
            Self::ClosedCriticalStream => 0x0104,
            Self::FrameUnexpected => 0x0105,
            Self::Frame => 0x0106,
            Self::ExcessiveLoad => 0x0107,
            Self::ID => 0x0108,
            Self::Settings => 0x0109,
            Self::MissingSettings => 0x010a,
            Self::RequestRejected => 0x010b,
            Self::RequestCancelled => 0x010c,
            Self::RequestIncomplete => 0x010d,
            Self::Message => 0x010e,
            Self::Connect => 0x010f,
            Self::VersionFallback => 0x0110,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
pub enum HttpError {
    Protocol(Error),
    QPack(crate::qpack::QPackError),
    Transport(quiche_tokio::ConnectionError),
}

impl HttpError {
    pub fn is_eof(&self) -> bool {
        if let Self::Transport(quiche_tokio::ConnectionError::Io(err)) = &self {
            *err == std::io::ErrorKind::UnexpectedEof
        } else {
            false
        }
    }

    pub fn to_id(&self) -> u64 {
        match self {
            Self::Protocol(err) => err.to_id(),
            Self::QPack(err) => err.to_id(),
            Self::Transport(err) => err.to_id(),
        }
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => f.write_fmt(format_args!("Transport({})", e)),
            Self::QPack(e) => f.write_fmt(format_args!("QPack({})", e)),
            Self::Protocol(e) => f.write_fmt(format_args!("Protocol({})", e)),
        }
    }
}

impl std::error::Error for HttpError {}

impl From<Error> for HttpError {
    fn from(value: Error) -> Self {
        Self::Protocol(value)
    }
}

impl From<crate::qpack::QPackError> for HttpError {
    fn from(value: crate::qpack::QPackError) -> Self {
        Self::QPack(value)
    }
}

impl From<quiche_tokio::ConnectionError> for HttpError {
    fn from(value: quiche_tokio::ConnectionError) -> Self {
        Self::Transport(value)
    }
}

impl From<std::io::Error> for HttpError {
    fn from(value: std::io::Error) -> Self {
        Self::Transport(quiche_tokio::ConnectionError::Io(value.kind()))
    }
}

pub type HttpResult<T> = Result<T, HttpError>;
