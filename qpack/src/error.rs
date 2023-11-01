#[derive(Debug)]
pub enum QPackError {
    DecompressionFailed,
    EncoderStreamError,
    DecoderStreamError,
}

impl QPackError {
    pub fn to_id(&self) -> u64 {
        match self {
            Self::DecompressionFailed => 0x0200,
            Self::EncoderStreamError => 0x0201,
            Self::DecoderStreamError => 0x0202,
        }
    }
}

impl std::fmt::Display for QPackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl std::error::Error for QPackError {}

pub type QPackResult<T> = Result<T, QPackError>;
