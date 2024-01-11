use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
pub struct BDPToken {
    pub saved_capacity: u64,
    pub saved_rtt: u64,
    pub address_validation_data: Vec<u8>,
    pub bdp_data: Vec<u8>
}

impl Default for BDPToken {
    fn default() -> Self {
        Self {
            saved_capacity: 0,
            saved_rtt: 0,
            address_validation_data: Vec::new(),
            bdp_data: Vec::new()
        }
    }
}

impl BDPToken {
    pub async fn encode<W: tokio::io::AsyncWrite + Unpin>(&self, buf: &mut W) -> std::io::Result<()> {
        quiver_util::vli::write_int(buf, self.address_validation_data.len() as u64).await?;
        buf.write_all(&self.address_validation_data).await?;

        quiver_util::vli::write_int(buf, self.bdp_data.len() as u64).await?;
        buf.write_all(&self.bdp_data).await?;

        quiver_util::vli::write_int(buf, self.saved_capacity).await?;
        quiver_util::vli::write_int(buf, self.saved_rtt).await?;
        Ok(())
    }

    pub async fn decode<R: tokio::io::AsyncRead + Unpin>(buf: &mut R) -> std::io::Result<Self> {
        let address_validation_data_len = quiver_util::vli::read_int(buf).await?;
        let mut address_validation_data = vec![0u8; address_validation_data_len as usize];
        buf.read_exact(&mut address_validation_data).await?;

        let bdp_data_len = quiver_util::vli::read_int(buf).await?;
        let mut bdp_data = vec![0u8; bdp_data_len as usize];
        buf.read_exact(&mut bdp_data).await?;

        let saved_capacity = quiver_util::vli::read_int(buf).await?;
        let saved_rtt = quiver_util::vli::read_int(buf).await?;

        Ok(Self {
            address_validation_data,
            bdp_data,
            saved_capacity,
            saved_rtt
        })
    }
}