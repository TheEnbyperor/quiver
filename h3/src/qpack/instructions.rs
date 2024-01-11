use super::{field_lines, util, PDU};

#[derive(Debug)]
pub enum EncoderInstruction {
    SetDynamicTableCapacity(u64),
    InsertNameReference {
        name_index: field_lines::FieldIndex,
        value: field_lines::FieldString,
    },
    InsertLiteralName {
        name: field_lines::FieldString,
        value: field_lines::FieldString,
    },
    Duplicate(u64),
}

#[async_trait::async_trait]
impl PDU for EncoderInstruction {
    async fn encode<W: tokio::io::AsyncWrite + Send + Sync + Unpin>(
        &self,
        buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<()> {
        use tokio_bitstream_io::BitWrite;
        match self {
            Self::SetDynamicTableCapacity(c) => {
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(true).await?;
                util::encode_integer_async(buf, 5, *c).await?;
            }
            Self::InsertNameReference { name_index, value } => {
                buf.write_bit(true).await?;
                name_index.encode(buf, 6).await?;
                value.encode(buf, 7).await?;
            }
            Self::InsertLiteralName { name, value } => {
                buf.write_bit(false).await?;
                buf.write_bit(true).await?;
                name.encode(buf, 5).await?;
                value.encode(buf, 7).await?;
            }
            Self::Duplicate(i) => {
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                util::encode_integer_async(buf, 5, *i).await?;
            }
        }
        Ok(())
    }

    async fn decode<R: tokio::io::AsyncRead + Send + Sync + Unpin>(
        buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
    ) -> crate::error::HttpResult<Option<Self>> {
        use tokio_bitstream_io::BitRead;
        if match crate::util::handle_http_io_error(buf.read_bit().await)? {
            Some(b) => b,
            None => return Ok(None)
        } {
            // Insert with Name Reference - prefix 1
            let name_index = field_lines::FieldIndex::decode(buf, 6).await?;
            let value = field_lines::FieldString::decode(buf, 7).await?;
            Ok(Some(Self::InsertNameReference { name_index, value }))
        } else if buf.read_bit().await? {
            // Insert with Literal Name - prefix 01
            let name = field_lines::FieldString::decode(buf, 5).await?;
            let value = field_lines::FieldString::decode(buf, 7).await?;
            Ok(Some(Self::InsertLiteralName { name, value }))
        } else if buf.read_bit().await? {
            // Set Dynamic Table Capacity - prefix 001
            let c = util::decode_integer_async(buf, 5).await?;
            Ok(Some(Self::SetDynamicTableCapacity(c)))
        } else {
            // Duplicate - prefix 000
            let i = util::decode_integer_async(buf, 5).await?;
            Ok(Some(Self::Duplicate(i)))
        }
    }
}

#[derive(Debug)]
pub enum DecoderInstruction {
    SectionAcknowledgment(u64),
    StreamCancellation(u64),
    InsertCountIncrement(u64),
}

#[async_trait::async_trait]
impl PDU for DecoderInstruction {
    async fn encode<W: tokio::io::AsyncWrite + Send + Sync + Unpin>(
        &self,
        buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
    ) -> std::io::Result<()> {
        use tokio_bitstream_io::BitWrite;
        match self {
            Self::SectionAcknowledgment(i) => {
                buf.write_bit(true).await?;
                util::encode_integer_async(buf, 7, *i).await?;
            }
            Self::StreamCancellation(i) => {
                buf.write_bit(false).await?;
                buf.write_bit(true).await?;
                util::encode_integer_async(buf, 6, *i).await?;
            }
            Self::InsertCountIncrement(i) => {
                buf.write_bit(false).await?;
                buf.write_bit(false).await?;
                util::encode_integer_async(buf, 6, *i).await?;
            }
        }
        Ok(())
    }

    async fn decode<R: tokio::io::AsyncRead + Send + Sync + Unpin>(
        buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
    ) -> crate::error::HttpResult<Option<Self>> {
        use tokio_bitstream_io::BitRead;
        if match crate::util::handle_http_io_error(buf.read_bit().await)? {
            Some(b) => b,
            None => return Ok(None)
        } {
            // Section Acknowledgment - prefix 1
            let i = util::decode_integer_async(buf, 7).await?;
            Ok(Some(Self::SectionAcknowledgment(i)))
        } else if buf.read_bit().await? {
            // Stream Cancellation - prefix 01
            let i = util::decode_integer_async(buf, 6).await?;
            Ok(Some(Self::StreamCancellation(i)))
        } else {
            // Insert Count Increment - prefix 00
            let i = util::decode_integer_async(buf, 6).await?;
            Ok(Some(Self::InsertCountIncrement(i)))
        }
    }
}

#[tokio::test]
async fn test_set_dynamic_table_capacity() {
    let data = hex::decode("3fbd01").unwrap();
    let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(&data));
    let encoder_instruction = EncoderInstruction::decode(&mut reader).await.unwrap();
    if let EncoderInstruction::SetDynamicTableCapacity(c) = encoder_instruction {
        assert_eq!(c, 220);
    } else {
        panic!("Expected SetDynamicTableCapacity");
    }

    let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
    encoder_instruction.encode(&mut writer).await.unwrap();
    let encoded = writer.into_writer().into_inner();
    assert_eq!(data, encoded);
}

#[tokio::test]
async fn test_insert_with_name_reference_static_table() {
    let data = hex::decode("c00f7777772e6578616d706c652e636f6d").unwrap();
    let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(&data));
    let encoder_instruction = EncoderInstruction::decode(&mut reader).await.unwrap();
    if let EncoderInstruction::InsertNameReference { name_index, value } = &encoder_instruction {
        if let field_lines::FieldIndex::Static(i) = name_index {
            assert_eq!(
                super::static_table::get_entry(*i).unwrap().name,
                b":authority"
            );
        } else {
            panic!("Expected static index");
        }
        if let field_lines::FieldString::Raw(v) = value {
            assert_eq!(v, b"www.example.com");
        } else {
            panic!("Expected raw string");
        }
    } else {
        panic!("Expected InsertNameReference");
    }

    let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
    encoder_instruction.encode(&mut writer).await.unwrap();
    let encoded = writer.into_writer().into_inner();
    assert_eq!(data, encoded);
}

#[tokio::test]
async fn test_insert_with_literal_name() {
    let data = hex::decode("4a637573746f6d2d6b65790c637573746f6d2d76616c7565").unwrap();
    let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(&data));
    let encoder_instruction = EncoderInstruction::decode(&mut reader).await.unwrap();
    if let EncoderInstruction::InsertLiteralName { name, value } = &encoder_instruction {
        if let field_lines::FieldString::Raw(v) = name {
            assert_eq!(v, b"custom-key");
        } else {
            panic!("Expected raw string");
        }
        if let field_lines::FieldString::Raw(v) = value {
            assert_eq!(v, b"custom-value");
        } else {
            panic!("Expected raw string");
        }
    } else {
        panic!("Expected InsertNameReference");
    }

    let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
    encoder_instruction.encode(&mut writer).await.unwrap();
    let encoded = writer.into_writer().into_inner();
    assert_eq!(data, encoded);
}

#[tokio::test]
async fn test_section_acknowledgement() {
    let data = hex::decode("84").unwrap();
    let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(&data));
    let decoder_instruction = DecoderInstruction::decode(&mut reader).await.unwrap();
    if let DecoderInstruction::SectionAcknowledgment(c) = decoder_instruction {
        assert_eq!(c, 4);
    } else {
        panic!("Expected SectionAcknowledgement");
    }

    let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
    decoder_instruction.encode(&mut writer).await.unwrap();
    let encoded = writer.into_writer().into_inner();
    assert_eq!(data, encoded);
}

#[tokio::test]
async fn test_insert_count_increment() {
    let data = hex::decode("01").unwrap();
    let mut reader = tokio_bitstream_io::BitReader::new(std::io::Cursor::new(&data));
    let decoder_instruction = DecoderInstruction::decode(&mut reader).await.unwrap();
    if let DecoderInstruction::InsertCountIncrement(c) = decoder_instruction {
        assert_eq!(c, 1);
    } else {
        panic!("Expected InsertCountIncrement");
    }

    let mut writer = tokio_bitstream_io::BitWriter::new(std::io::Cursor::new(vec![]));
    decoder_instruction.encode(&mut writer).await.unwrap();
    let encoded = writer.into_writer().into_inner();
    assert_eq!(data, encoded);
}
