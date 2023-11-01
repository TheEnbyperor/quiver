use super::{huffman, util, PDU};

#[derive(Debug, Clone)]
pub struct FieldLines {
    pub(super) required_insert_count: u64,
    pub(super) delta_base: i64,
    pub(super) lines: Vec<FieldLine>,
}

#[derive(Debug, Clone)]
pub enum FieldLine {
    Indexed(FieldIndex),
    PostBaseIndexed(u64),
    LiteralWithNameReference {
        must_be_literal: bool,
        name_index: FieldIndex,
        value: FieldString,
    },
    LiteralWithPostBaseNameReference {
        must_be_literal: bool,
        name_index: u64,
        value: FieldString,
    },
    Literal {
        must_be_literal: bool,
        name: FieldString,
        value: FieldString,
    },
}

#[derive(Copy, Clone)]
pub enum FieldIndex {
    Static(u64),
    Dynamic(u64),
}

impl std::fmt::Debug for FieldIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static(i) => f.write_fmt(format_args!(
                "Static({}, field={:?})",
                i,
                super::static_table::get_entry(*i)
            )),
            Self::Dynamic(i) => f.write_fmt(format_args!("Dynamic({}", i)),
        }
    }
}

impl FieldIndex {
    pub(super) async fn encode<W: tokio::io::AsyncWrite + Sync + Send + Unpin>(
        &self,
        buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
        n: u32,
    ) -> std::io::Result<()> {
        use tokio_bitstream_io::BitWrite;
        match self {
            Self::Static(i) => {
                buf.write_bit(true).await?;
                util::encode_integer_async(buf, n, *i).await?;
            }
            Self::Dynamic(i) => {
                buf.write_bit(false).await?;
                util::encode_integer_async(buf, n, *i).await?;
            }
        }
        Ok(())
    }
    pub(super) async fn decode<R: tokio::io::AsyncRead + Sync + Send + Unpin>(
        buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
        n: u32,
    ) -> std::io::Result<Self> {
        use tokio_bitstream_io::BitRead;
        let is_static = buf.read_bit().await?;
        let index = util::decode_integer_async(buf, n).await?;
        if is_static {
            Ok(Self::Static(index))
        } else {
            Ok(Self::Dynamic(index))
        }
    }
}

#[derive(Clone)]
pub enum FieldString {
    Raw(Vec<u8>),
    Huffman(Vec<u8>),
}

impl std::fmt::Debug for FieldString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw(s) => f.write_fmt(format_args!(
                "Raw(utf8={:?}, data={:?})",
                String::from_utf8_lossy(&s),
                s
            )),
            Self::Huffman(s) => f.write_fmt(format_args!("Huffman({:?})", s)),
        }
    }
}

impl FieldString {
    pub fn to_vec(self) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Raw(r) => Ok(r),
            Self::Huffman(h) => huffman::decode(&h),
        }
    }
}

#[async_trait::async_trait]
impl PDU for FieldLines {
    async fn encode<W: tokio::io::AsyncWrite + Send + Sync + Unpin>(
        &self,
        buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<()> {
        use tokio_bitstream_io::BitWrite;
        util::encode_integer_async(buf, 8, self.required_insert_count).await?;
        if self.delta_base < 0 {
            buf.write_bit(true).await?;
            util::encode_integer_async(buf, 7, (self.delta_base.abs() as u64) - 1).await?;
        } else {
            buf.write_bit(false).await?;
            util::encode_integer_async(buf, 7, self.delta_base as u64).await?;
        }
        for line in &self.lines {
            line.encode(buf).await?;
        }
        Ok(())
    }

    async fn decode<R: tokio::io::AsyncRead + Send + Sync + Unpin>(
        buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<Self> {
        use tokio_bitstream_io::BitRead;
        let required_insert_count = util::decode_integer_async(buf, 8).await?;
        let delta_base_sign = buf.read_bit().await?;
        let delta_base_abs = util::decode_integer_async(buf, 7).await? as i64;
        let delta_base = if delta_base_sign {
            -delta_base_abs - 1
        } else {
            delta_base_abs
        };
        let mut lines = vec![];
        while let Some(line) = FieldLine::decode(buf).await? {
            lines.push(line);
        }
        Ok(Self {
            required_insert_count,
            delta_base,
            lines,
        })
    }
}

impl FieldLine {
    async fn encode<W: tokio::io::AsyncWrite + Send + Sync + Unpin>(
        &self,
        buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<()> {
        use tokio_bitstream_io::BitWrite;
        match self {
            Self::Indexed(i) => {
                buf.write_bit(true).await?;
                i.encode(buf, 6).await?;
            }
            Self::PostBaseIndexed(i) => {
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(true).await?;
                util::encode_integer_async(buf, 4, *i).await?;
            }
            Self::LiteralWithNameReference {
                must_be_literal,
                name_index,
                value,
            } => {
                buf.write_bit(false).await?;
                buf.write_bit(true).await?;
                buf.write_bit(*must_be_literal).await?;
                name_index.encode(buf, 4).await?;
                value.encode(buf, 7).await?;
            }
            Self::LiteralWithPostBaseNameReference {
                must_be_literal,
                name_index,
                value,
            } => {
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(*must_be_literal).await?;
                util::encode_integer_async(buf, 3, *name_index).await?;
                value.encode(buf, 7).await?;
            }
            Self::Literal {
                must_be_literal,
                name,
                value,
            } => {
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(true).await?;
                buf.write_bit(*must_be_literal).await?;
                name.encode(buf, 3).await?;
                value.encode(buf, 7).await?;
            }
        }

        Ok(())
    }

    async fn decode<R: tokio::io::AsyncRead + Send + Sync + Unpin>(
        buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<Option<Self>> {
        use tokio_bitstream_io::BitRead;
        let first_bit = match buf.read_bit().await {
            Ok(b) => b,
            Err(e) => {
                return if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    Ok(None)
                } else {
                    Err(e)
                };
            }
        };
        if first_bit {
            // Indexed Field Line - prefix 1
            let name_index = FieldIndex::decode(buf, 6).await?;
            Ok(Some(Self::Indexed(name_index)))
        } else if buf.read_bit().await? {
            // Literal Field Line with Name Reference - prefix 01
            let must_be_literal = buf.read_bit().await?;
            let name_index = FieldIndex::decode(buf, 4).await?;
            let value = FieldString::decode(buf, 7).await?;
            Ok(Some(Self::LiteralWithNameReference {
                must_be_literal,
                name_index,
                value,
            }))
        } else if buf.read_bit().await? {
            // Literal Field Line with Literal Name - prefix 001
            let must_be_literal = buf.read_bit().await?;
            let name = FieldString::decode(buf, 3).await?;
            let value = FieldString::decode(buf, 7).await?;
            Ok(Some(Self::Literal {
                must_be_literal,
                name,
                value,
            }))
        } else if buf.read_bit().await? {
            // Indexed Field Line with Post-Base Index - prefix 0001
            let i = util::decode_integer_async(buf, 4).await?;
            Ok(Some(Self::PostBaseIndexed(i)))
        } else {
            // Literal Field Line with Post-Base Name Reference - prefix 0000
            let must_be_literal = buf.read_bit().await?;
            let i = util::decode_integer_async(buf, 3).await?;
            let value = FieldString::decode(buf, 7).await?;
            Ok(Some(Self::LiteralWithPostBaseNameReference {
                must_be_literal,
                name_index: i,
                value,
            }))
        }
    }
}

impl FieldString {
    pub(super) async fn encode<W: tokio::io::AsyncWrite + Sync + Send + Unpin>(
        &self,
        buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
        n: u32,
    ) -> std::io::Result<()> {
        use tokio_bitstream_io::BitWrite;
        match self {
            Self::Raw(r) => {
                buf.write_bit(false).await?;
                util::encode_integer_async(buf, n, r.len() as u64).await?;
                buf.write_bytes(&r).await?;
            }
            Self::Huffman(r) => {
                buf.write_bit(true).await?;
                util::encode_integer_async(buf, n, r.len() as u64).await?;
                buf.write_bytes(&r).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn decode<R: tokio::io::AsyncRead + Sync + Send + Unpin>(
        buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
        n: u32,
    ) -> std::io::Result<Self> {
        use tokio_bitstream_io::BitRead;
        let is_huffman = buf.read_bit().await?;
        let len = util::decode_integer_async(buf, n).await?;
        let val = buf.read_to_vec(len as usize).await?;
        Ok(if is_huffman {
            Self::Huffman(val)
        } else {
            Self::Raw(val)
        })
    }
}

#[tokio::test]
async fn test_literal_field_line_with_name_reference() {
    let data = hex::decode("0000510b2f696e6465782e68746d6c").unwrap();
    let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(&data));
    let field_lines = FieldLines::decode(&mut reader).await.unwrap();

    assert_eq!(field_lines.required_insert_count, 0);
    assert_eq!(field_lines.delta_base, 0);
    assert_eq!(field_lines.lines.len(), 1);
    match &field_lines.lines[0] {
        FieldLine::LiteralWithNameReference {
            must_be_literal,
            name_index,
            value,
        } => {
            assert_eq!(*must_be_literal, false);
            if let FieldIndex::Static(i) = name_index {
                assert_eq!(super::static_table::get_entry(*i).unwrap().name, b":path");
            } else {
                panic!("Expected static index");
            }
            if let FieldString::Raw(v) = value {
                assert_eq!(v, b"/index.html");
            } else {
                panic!("Expected raw string");
            }
        }
        _ => panic!("Expected LiteralWithNameReference"),
    }

    let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
    field_lines.encode(&mut writer).await.unwrap();
    let encoded = writer.into_writer().into_inner();
    assert_eq!(data, encoded);
}

#[tokio::test]
async fn test_indexed_field_line_with_post_base_index() {
    let data = hex::decode("03811011").unwrap();
    let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(&data));
    let field_lines = FieldLines::decode(&mut reader).await.unwrap();

    assert_eq!(field_lines.required_insert_count, 3);
    assert_eq!(field_lines.delta_base, -2);
    assert_eq!(field_lines.lines.len(), 2);
    match &field_lines.lines[0] {
        FieldLine::PostBaseIndexed(i) => {
            assert_eq!(*i, 0)
        }
        _ => panic!("Expected PostBaseIndexed"),
    }
    match &field_lines.lines[1] {
        FieldLine::PostBaseIndexed(i) => {
            assert_eq!(*i, 1)
        }
        _ => panic!("Expected PostBaseIndexed"),
    }

    let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
    field_lines.encode(&mut writer).await.unwrap();
    let encoded = writer.into_writer().into_inner();
    assert_eq!(data, encoded);
}
