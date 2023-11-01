#![feature(extract_if)]

mod decoder;
mod dynamic_table;
mod encoder;
mod error;
mod field_lines;
mod huffman;
mod instructions;
mod static_table;
mod util;

pub use decoder::{DecodeResult, Decoder};
pub use encoder::Encoder;
pub use error::{QPackError, QPackResult};
pub use field_lines::FieldLines;
pub use instructions::{DecoderInstruction, EncoderInstruction};

#[derive(Debug, Clone)]
pub struct Headers<'a> {
    headers: Vec<Header<'a>>,
}

impl<'a> Default for Headers<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Headers<'a> {
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
        }
    }

    pub fn add_header(&mut self, header: Header<'a>) {
        self.headers.push(header);
    }

    pub fn add(&mut self, name: &'a [u8], value: &'a [u8]) {
        self.headers.push(Header {
            name: std::borrow::Cow::Borrowed(name),
            value: std::borrow::Cow::Borrowed(value),
        });
    }
}

impl<'a> std::ops::Deref for Headers<'a> {
    type Target = [Header<'a>];

    fn deref(&self) -> &Self::Target {
        self.headers.deref()
    }
}

#[derive(Clone)]
pub struct Header<'a> {
    pub name: std::borrow::Cow<'a, [u8]>,
    pub value: std::borrow::Cow<'a, [u8]>,
}

impl std::fmt::Debug for Header<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{}={}",
            String::from_utf8_lossy(&self.name),
            String::from_utf8_lossy(&self.value)
        ))
    }
}

impl Header<'static> {
    pub fn from_static(name: &'static [u8], value: &'static [u8]) -> Self {
        Self {
            name: std::borrow::Cow::Borrowed(name),
            value: std::borrow::Cow::Borrowed(value),
        }
    }

    pub fn from_bytes(name: Vec<u8>, value: Vec<u8>) -> Self {
        Self {
            name: std::borrow::Cow::Owned(name),
            value: std::borrow::Cow::Owned(value),
        }
    }
}

impl<'a> Header<'a> {
    fn make_static(&self) -> Header<'static> {
        Header {
            name: std::borrow::Cow::Owned(self.name.clone().into_owned()),
            value: std::borrow::Cow::Owned(self.value.clone().into_owned()),
        }
    }

    fn size(&self) -> u64 {
        self.name.len() as u64 + self.value.len() as u64 + 32u64
    }
}

#[async_trait::async_trait]
pub trait PDU: Sized {
    async fn encode<W: tokio::io::AsyncWrite + Send + Sync + Unpin>(
        &self,
        buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<()>;

    async fn decode<R: tokio::io::AsyncRead + Send + Sync + Unpin>(
        buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<Self>;

    async fn encode_bytes<W: tokio::io::AsyncWrite + Send + Sync + Unpin>(
        &self,
        buf: &mut W,
    ) -> std::io::Result<()> {
        let mut writer = tokio_bitstream_io::write::BitWriter::new(buf);
        self.encode(&mut writer).await
    }

    async fn decode_bytes<R: tokio::io::AsyncRead + Send + Sync + Unpin>(
        buf: &mut R,
    ) -> std::io::Result<Self> {
        let mut reader = tokio_bitstream_io::read::BitReader::new(buf);
        Self::decode(&mut reader).await
    }

    async fn to_vec(&self) -> Vec<u8> {
        let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
        self.encode(&mut writer).await.unwrap();
        writer.into_writer().into_inner()
    }

    async fn from_bytes(data: &[u8]) -> std::io::Result<Self> {
        let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(data));
        Self::decode(&mut reader).await
    }
}
