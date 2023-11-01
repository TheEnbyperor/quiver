#[derive(Debug)]
pub enum Error {
    GeneralProtocolError,
    InternalError,
    StreamCreationError,
    ClosedCriticalStream,
    FrameUnexpected,
    FrameError,
    ExcessiveLoad,
    IDError,
    SettingsError,
    MissingSettings,
    RequestRejected,
    RequestCancelled,
    RequestIncomplete,
    MessageError,
    ConnectError,
    VersionFallback,
}

impl Error {
    pub fn to_id(&self) -> u64 {
        match self {
            Self::GeneralProtocolError => 0x0101,
            Self::InternalError => 0x0102,
            Self::StreamCreationError => 0x0103,
            Self::ClosedCriticalStream => 0x0104,
            Self::FrameUnexpected => 0x0105,
            Self::FrameError => 0x0106,
            Self::ExcessiveLoad => 0x0107,
            Self::IDError => 0x0108,
            Self::SettingsError => 0x0109,
            Self::MissingSettings => 0x010a,
            Self::RequestRejected => 0x010b,
            Self::RequestCancelled => 0x010c,
            Self::RequestIncomplete => 0x010d,
            Self::MessageError => 0x010e,
            Self::ConnectError => 0x010f,
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
    ProtocolError(Error),
    QPackError(quiver_qpack::QPackError),
    TransportError(quiche_tokio::ConnectionError),
}

impl HttpError {
    pub fn is_eof(&self) -> bool {
        if let Self::TransportError(err) = &self {
            if let quiche_tokio::ConnectionError::Io(err) = err {
                *err == std::io::ErrorKind::UnexpectedEof
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn to_id(&self) -> u64 {
        match self {
            Self::ProtocolError(err) => err.to_id(),
            Self::QPackError(err) => err.to_id(),
            Self::TransportError(err) => err.to_id(),
        }
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TransportError(e) => f.write_fmt(format_args!("TransportError({})", e)),
            Self::QPackError(e) => f.write_fmt(format_args!("QPackError({})", e)),
            Self::ProtocolError(e) => f.write_fmt(format_args!("ProtocolError({})", e)),
        }
    }
}

impl std::error::Error for HttpError {}

impl From<Error> for HttpError {
    fn from(value: Error) -> Self {
        Self::ProtocolError(value)
    }
}

impl From<quiver_qpack::QPackError> for HttpError {
    fn from(value: quiver_qpack::QPackError) -> Self {
        Self::QPackError(value)
    }
}

impl From<quiche_tokio::ConnectionError> for HttpError {
    fn from(value: quiche_tokio::ConnectionError) -> Self {
        Self::TransportError(value)
    }
}

impl From<std::io::Error> for HttpError {
    fn from(value: std::io::Error) -> Self {
        Self::TransportError(quiche_tokio::ConnectionError::Io(value.kind()))
    }
}

pub type HttpResult<T> = Result<T, HttpError>;
