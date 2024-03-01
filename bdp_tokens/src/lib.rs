use std::io::{Read, Write};
use byteorder::{ReadBytesExt, WriteBytesExt};
use chacha20poly1305::{AeadCore, AeadInPlace, KeyInit};

const NANOS_PER_MICRO: u32 = 1_000;
const MICROS_PER_SEC: u128 = 1_000_000;

#[repr(u64)]
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, num_enum::FromPrimitive, num_enum::IntoPrimitive)]
pub enum ExtensionType {
    BDPToken = 1,
    #[num_enum(catch_all)]
    Unknown(u64)
}

#[derive(Debug, Clone)]
pub struct ExToken {
    pub address_validation_data: Vec<u8>,
    pub extensions: std::collections::BTreeMap<ExtensionType, Vec<u8>>
}

impl Default for ExToken {
    fn default() -> Self {
        Self {
            address_validation_data: Vec::new(),
            extensions: std::collections::BTreeMap::new()
        }
    }
}

impl ExToken {
    pub fn get_extension(&self, extension_type: ExtensionType) -> Option<&[u8]> {
        self.extensions.get(&extension_type).map(|e| e.as_slice())
    }

    pub fn encode<W: Write + Unpin>(&self, buf: &mut W) -> std::io::Result<()> {
        quiver_util::vli::write_int(buf, self.address_validation_data.len() as u64)?;
        buf.write_all(&self.address_validation_data)?;

        for (k, v) in self.extensions.iter() {
            quiver_util::vli::write_int(buf, u64::from(*k))?;
            quiver_util::vli::write_int(buf, v.len())?;
            buf.write_all(v)?;
        }

        Ok(())
    }

    pub fn decode<R: Read + Unpin>(buf: &mut R) -> std::io::Result<Self> {
        let address_validation_data_len: usize = quiver_util::vli::read_int(buf)?;
        let mut address_validation_data = vec![0u8; address_validation_data_len];
        buf.read_exact(&mut address_validation_data)?;

        let mut extensions = std::collections::BTreeMap::new();

        loop {
            let ext_id = match quiver_util::vli::read_int::<u64, _>(buf) {
                Ok(i) => i.into(),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        break
                    } else {
                        return Err(e);
                    }
                }
            };

            let ext_len: usize = quiver_util::vli::read_int(buf)?;
            let mut ext_data = vec![0u8; ext_len];
            buf.read_exact(&mut ext_data)?;

            if extensions.insert(ext_id, ext_data).is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData, "Duplicate extension"
                ));
            }
        }

        Ok(Self {
            address_validation_data,
            extensions
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BDPToken {
    pub saved_capacity: u64,
    pub saved_rtt: u128,
    pub requested_capacity: u64,
    pub private_data: Vec<u8>
}

impl Default for BDPToken {
    fn default() -> Self {
        Self {
            saved_capacity: 0,
            saved_rtt: 0,
            requested_capacity: 0,
            private_data: Vec::new()
        }
    }
}

impl BDPToken {
    pub fn saved_rtt_duration(&self) -> std::time::Duration {
        std::time::Duration::new(
            (self.saved_rtt / MICROS_PER_SEC) as u64,
            ((self.saved_rtt % MICROS_PER_SEC) as u32) * NANOS_PER_MICRO
        )
    }

    pub fn encode<W: Write + Unpin>(&self, buf: &mut W) -> std::io::Result<()> {
        quiver_util::vli::write_int(buf, self.private_data.len() as u64)?;
        buf.write_all(&self.private_data)?;

        quiver_util::vli::write_int(buf, self.saved_capacity)?;
        quiver_util::vli::write_int(buf, self.saved_rtt)?;
        quiver_util::vli::write_int(buf, self.requested_capacity)?;
        Ok(())
    }

    pub fn decode<R: Read + Unpin>(buf: &mut R) -> std::io::Result<Self> {
        let private_data_len: usize = quiver_util::vli::read_int(buf)?;
        let mut private_data = vec![0u8; private_data_len];
        buf.read_exact(&mut private_data)?;

        let saved_capacity = quiver_util::vli::read_int(buf)?;
        let saved_rtt = quiver_util::vli::read_int(buf)?;
        let requested_capacity = quiver_util::vli::read_int(buf)?;

        Ok(Self {
            private_data,
            saved_capacity,
            saved_rtt,
            requested_capacity,
        })
    }

    pub fn from_quiche_cr_event(
        event: &quiche::CREvent,
        ip: std::net::IpAddr,
        lifetime: chrono::Duration,
        key: &[u8; 32]
    ) -> Self {
        let mut out = Self {
            saved_capacity: event.cwnd as u64,
            requested_capacity: event.cwnd as u64,
            saved_rtt: event.min_rtt.as_micros(),
            private_data: Vec::new()
        };

        let expiry = chrono::Utc::now() + lifetime;

        let private_data = PrivateBDPData {
            ip,
            expiry,
        };
        private_data.encrypt(&mut out, key);

        out
    }

    pub fn private_data(
        &mut self,
        key: &[u8; 32]
    ) -> Option<PrivateBDPData> {
        PrivateBDPData::decrypt(self, key)
    }
}

#[derive(Debug)]
pub struct PrivateBDPData {
    ip: std::net::IpAddr,
    expiry: chrono::DateTime<chrono::Utc>,
}

impl PrivateBDPData {
    pub fn ip(&self) -> std::net::IpAddr {
        self.ip
    }

    pub fn expiry(&self) -> chrono::DateTime<chrono::Utc> {
        self.expiry
    }

    pub fn expired(&self) -> bool {
        self.expiry < chrono::Utc::now()
    }

    fn associated_data(bdp_token: &BDPToken) -> Vec<u8> {
        let mut associated_data = std::io::Cursor::new(Vec::new());
        associated_data.write_all(&bdp_token.saved_capacity.to_be_bytes()).unwrap();
        associated_data.write_all(&bdp_token.saved_rtt.to_be_bytes()).unwrap();
        associated_data.into_inner()
    }

    fn encrypt(&self, bdp_token: &mut BDPToken, key: &[u8; 32]) {
        let cipher = chacha20poly1305::ChaCha20Poly1305::new(key.into());

        let mut data = std::io::Cursor::new(&mut bdp_token.private_data);

        match self.ip {
            std::net::IpAddr::V4(ip) => {
                data.write_u8(0x04).unwrap();
                data.write_all(&ip.octets()).unwrap();
            }
            std::net::IpAddr::V6(ip) => {
                data.write_u8(0x06).unwrap();
                data.write_all(&ip.octets()).unwrap();
            }
        }

        data.write_all(&self.expiry.timestamp().to_be_bytes()).unwrap();

        let nonce = chacha20poly1305::ChaCha20Poly1305::generate_nonce(
            &mut chacha20poly1305::aead::OsRng
        );
        cipher.encrypt_in_place(
            &nonce, &Self::associated_data(bdp_token), &mut bdp_token.private_data
        ).unwrap();
        bdp_token.private_data.extend_from_slice(nonce.as_slice());
    }

    fn decrypt(bdp_token: &mut BDPToken, key: &[u8; 32]) -> Option<Self> {
        let cipher = chacha20poly1305::ChaCha20Poly1305::new(key.into());

        let data_len = bdp_token.private_data.len();
        if data_len < 12 {
            return None;
        }
        let nonce = <[u8; 12]>::try_from(bdp_token.private_data.drain(data_len-12..data_len)
            .collect::<Vec<_>>()).unwrap();

        if let Err(_) = cipher.decrypt_in_place(
            &nonce.into(), &Self::associated_data(bdp_token), &mut bdp_token.private_data
        ) {
            return None;
        }

        let mut data = std::io::Cursor::new(&bdp_token.private_data);

        let ip_ver = data.read_u8().ok()?;
        let ip = match ip_ver {
            0x04 => {
                let mut subnet = [0u8; 4];
                data.read_exact(&mut subnet).ok()?;
                std::net::IpAddr::V4(std::net::Ipv4Addr::from(subnet))
            }
            0x06 => {
                let mut subnet = [0u8; 16];
                data.read_exact(&mut subnet).ok()?;
                std::net::IpAddr::V6(std::net::Ipv6Addr::from(subnet))
            }
            _ => return None
        };

        let mut expiry_timestamp_bytes = [0u8; 8];
        data.read_exact(&mut expiry_timestamp_bytes).ok()?;
        let expiry_timestamp = i64::from_be_bytes(expiry_timestamp_bytes);

        let expiry = chrono::DateTime::from_timestamp(expiry_timestamp, 0)?;

        Some(Self {
            ip,
            expiry,
        })
    }
}