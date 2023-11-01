use super::{error, settings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
pub enum Frame {
    Data(Vec<u8>),
    Headers(Vec<u8>),
    CancelPush(u64),
    Settings(settings::Settings),
    PushPromise { push_id: u64, field_lines: Vec<u8> },
    GoAway(u64),
    MaxPushID(u64),
    Unknown { frame_type: u64, data: Vec<u8> },
}

impl Frame {
    pub async fn write<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        buf: &mut W,
    ) -> error::HttpResult<()> {
        match self {
            Self::Data(data) => {
                crate::vli::write_int(buf, 0x00).await?;
                crate::vli::write_int(buf, data.len() as u64).await?;
                buf.write_all(data).await?;
                Ok(())
            }
            Self::Headers(data) => {
                crate::vli::write_int(buf, 0x01).await?;
                crate::vli::write_int(buf, data.len() as u64).await?;
                buf.write_all(data).await?;
                Ok(())
            }
            Self::CancelPush(push_id) => {
                let mut data = std::io::Cursor::new(Vec::new());
                crate::vli::write_int(&mut data, *push_id).await?;
                let data = data.into_inner();

                crate::vli::write_int(buf, 0x03).await?;
                crate::vli::write_int(buf, data.len() as u64).await?;
                buf.write_all(&data).await?;

                Ok(())
            }
            Self::Settings(settings) => {
                let data = settings.to_vec();
                crate::vli::write_int(buf, 0x04).await?;
                crate::vli::write_int(buf, data.len() as u64).await?;
                buf.write_all(&data).await?;
                Ok(())
            }
            Self::PushPromise {
                push_id,
                field_lines,
            } => {
                let mut data = std::io::Cursor::new(Vec::new());
                crate::vli::write_int(&mut data, *push_id).await?;
                let data = data.into_inner();

                crate::vli::write_int(buf, 0x05).await?;
                crate::vli::write_int(buf, data.len() as u64 + field_lines.len() as u64).await?;
                buf.write_all(&data).await?;
                buf.write_all(field_lines).await?;

                Ok(())
            }
            Self::GoAway(stream_id) => {
                let mut data = std::io::Cursor::new(Vec::new());
                crate::vli::write_int(&mut data, *stream_id).await?;
                let data = data.into_inner();

                crate::vli::write_int(buf, 0x07).await?;
                crate::vli::write_int(buf, data.len() as u64).await?;
                buf.write_all(&data).await?;

                Ok(())
            }
            Self::MaxPushID(max_push_id) => {
                let mut data = std::io::Cursor::new(Vec::new());
                crate::vli::write_int(&mut data, *max_push_id).await?;
                let data = data.into_inner();

                crate::vli::write_int(buf, 0x0d).await?;
                crate::vli::write_int(buf, data.len() as u64).await?;
                buf.write_all(&data).await?;

                Ok(())
            }
            Self::Unknown { frame_type, data } => {
                crate::vli::write_int(buf, *frame_type).await?;
                crate::vli::write_int(buf, data.len() as u64).await?;
                buf.write_all(data).await?;
                Ok(())
            }
        }
    }

    pub async fn read<R: tokio::io::AsyncRead + Unpin>(
        buf: &mut R,
    ) -> error::HttpResult<Option<Self>> {
        let frame_type = match crate::vli::read_int(buf).await {
            Ok(t) => t,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Ok(None);
                }
                return Err(err.into());
            }
        };
        let length: usize = crate::vli::read_int(buf)
            .await?
            .try_into()
            .map_err(|_| error::Error::Frame)?;
        let mut data = vec![0u8; length];
        buf.read_exact(&mut data).await?;
        Ok(Some(match frame_type {
            0x00 => Self::Data(data),
            0x01 => Self::Headers(data),
            0x03 => {
                let mut cur = std::io::Cursor::new(data);
                let stream_id = crate::vli::read_int(&mut cur).await?;
                if cur.position() as usize != length {
                    return Err(error::Error::Frame.into());
                }
                Self::CancelPush(stream_id)
            }
            0x04 => {
                let mut cur = std::io::Cursor::new(data);
                let settings = settings::Settings::read(&mut cur).await?;
                if cur.position() as usize != length {
                    return Err(error::Error::Frame.into());
                }
                Self::Settings(settings)
            }
            0x05 => {
                let mut cur = std::io::Cursor::new(data);
                let push_id = crate::vli::read_int(&mut cur).await?;
                let pos = cur.position() as usize;
                let field_lines = cur.into_inner()[pos..].to_vec();
                Self::PushPromise {
                    push_id,
                    field_lines,
                }
            }
            0x07 => {
                let mut cur = std::io::Cursor::new(data);
                let stream_id = crate::vli::read_int(&mut cur).await?;
                if cur.position() as usize != length {
                    return Err(error::Error::Frame.into());
                }
                Self::GoAway(stream_id)
            }
            0x0d => {
                let mut cur = std::io::Cursor::new(data);
                let max_push_id = crate::vli::read_int(&mut cur).await?;
                if cur.position() as usize != length {
                    return Err(error::Error::Frame.into());
                }
                Self::MaxPushID(max_push_id)
            }
            o => Self::Unknown {
                frame_type: o,
                data,
            },
        }))
    }
}
