use super::error;

#[derive(Debug, Default, Clone)]
pub struct Settings {
    qpack_max_table_capacity: Option<u64>,
    qpack_blocked_streams: Option<u64>,
    max_field_section_size: Option<u64>,
    other_settings: Vec<(u64, u64)>,
}

impl Settings {
    pub fn qpack_max_table_capacity(&self) -> u64 {
        self.qpack_max_table_capacity.unwrap_or(0)
    }

    // pub fn qpack_blocked_streams(&self) -> u64 {
    //     self.qpack_blocked_streams.unwrap_or(0)
    // }
}

impl Settings {
    pub fn to_vec(&self) -> Vec<u8> {
        vec![]
    }

    pub async fn read<R: tokio::io::AsyncRead + Unpin>(buf: &mut R) -> error::HttpResult<Self> {
        let mut out = Self::default();
        loop {
            let identifier = match crate::vli::read_int(buf).await {
                Ok(i) => i,
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::UnexpectedEof {
                        break;
                    }
                    return Err(error::Error::Frame.into());
                }
            };
            let value = crate::vli::read_int(buf)
                .await
                .map_err(|_| error::Error::Frame)?;

            match identifier {
                0x01 => {
                    if out.qpack_max_table_capacity.is_some() {
                        return Err(error::Error::Settings.into());
                    }
                    out.qpack_max_table_capacity = Some(value)
                }
                0x06 => {
                    if out.max_field_section_size.is_some() {
                        return Err(error::Error::Settings.into());
                    }
                    out.max_field_section_size = Some(value)
                }
                0x07 => {
                    if out.qpack_blocked_streams.is_some() {
                        return Err(error::Error::Settings.into());
                    }
                    out.qpack_blocked_streams = Some(value)
                }
                o => {
                    out.other_settings.push((o, value));
                }
            }
        }
        Ok(out)
    }
}
