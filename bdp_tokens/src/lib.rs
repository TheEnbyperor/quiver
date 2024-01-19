use std::io::{Read, Write};
use digest::generic_array;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use hmac::digest::OutputSizeUser;

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

const NANOS_PER_MICRO: u32 = 1_000;
const MICROS_PER_SEC: u128 = 1_000_000;

#[derive(Debug, Clone)]
pub struct BDPToken {
    pub saved_capacity: u64,
    pub saved_rtt: u128,
    pub requested_capacity: u64,
    pub address_validation_data: Vec<u8>,
    pub bdp_data: Vec<u8>
}

impl Default for BDPToken {
    fn default() -> Self {
        Self {
            saved_capacity: 0,
            saved_rtt: 0,
            requested_capacity: 0,
            address_validation_data: Vec::new(),
            bdp_data: Vec::new()
        }
    }
}

impl BDPToken {
    pub fn encode<W: Write + Unpin>(&self, buf: &mut W) -> std::io::Result<()> {
        quiver_util::vli::write_int(buf, self.address_validation_data.len() as u64)?;
        buf.write_all(&self.address_validation_data)?;

        quiver_util::vli::write_int(buf, self.bdp_data.len() as u64)?;
        buf.write_all(&self.bdp_data)?;

        quiver_util::vli::write_int(buf, self.saved_capacity)?;
        quiver_util::vli::write_int(buf, self.saved_rtt)?;
        quiver_util::vli::write_int(buf, self.requested_capacity)?;
        Ok(())
    }

    pub fn decode<R: std::io::Read + Unpin>(buf: &mut R) -> std::io::Result<Self> {
        let address_validation_data_len: usize = quiver_util::vli::read_int(buf)?;
        let mut address_validation_data = vec![0u8; address_validation_data_len];
        buf.read_exact(&mut address_validation_data)?;

        let bdp_data_len: usize = quiver_util::vli::read_int(buf)?;
        let mut bdp_data = vec![0u8; bdp_data_len];
        buf.read_exact(&mut bdp_data)?;

        let saved_capacity = quiver_util::vli::read_int(buf)?;
        let saved_rtt = quiver_util::vli::read_int(buf)?;
        let requested_capacity = quiver_util::vli::read_int(buf)?;

        Ok(Self {
            address_validation_data,
            bdp_data,
            saved_capacity,
            saved_rtt,
            requested_capacity,
        })
    }
}

#[derive(Debug)]
pub struct CRBDPData {
    saved_capacity: u64,
    saved_rtt: std::time::Duration,
    ip: std::net::IpAddr,
    expiry: chrono::DateTime<chrono::Utc>,
    signature: Vec<u8>
}

impl CRBDPData {
    pub fn saved_capacity(&self) -> u64 {
        self.saved_capacity
    }

    pub fn saved_rtt(&self) -> std::time::Duration {
        self.saved_rtt
    }

    pub fn ip(&self) -> std::net::IpAddr {
        self.ip
    }

    pub fn expiry(&self) -> chrono::DateTime<chrono::Utc> {
        self.expiry
    }

    pub fn expired(&self) -> bool {
        self.expiry < chrono::Utc::now()
    }

    pub fn from_quiche_cr_event(
        event: &quiche::CREvent,
        ip: std::net::IpAddr,
        lifetime: chrono::Duration,
        key: &[u8]
    ) -> Self {
        let now = chrono::Utc::now();
        let expiry = now + lifetime;

        let mut out = Self {
            saved_capacity: event.cwnd as u64,
            saved_rtt: event.min_rtt,
            ip,
            expiry,
            signature: vec![],
        };

        out.signature = out.make_mac(key).into_bytes().to_vec();

        out
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());

        let expiry_timestamp = self.expiry.timestamp();

        out.write_all(&self.saved_capacity.to_be_bytes()).unwrap();
        out.write_all(&self.saved_rtt.as_micros().to_be_bytes()).unwrap();

        match self.ip {
            std::net::IpAddr::V4(ip) => {
                out.write_u8(0x04).unwrap();
                // Only write the /24 subnet
                out.write_all(&ip.octets()[..3]).unwrap();
            }
            std::net::IpAddr::V6(ip) => {
                out.write_u8(0x06).unwrap();
                // Only write the /48 subnet
                out.write_all(&ip.octets()[..6]).unwrap();
            }
        }

        out.write_all(&expiry_timestamp.to_be_bytes()).unwrap();
        out.write_all(&self.signature).unwrap();

        out.into_inner()
    }

    pub fn from_bytes(data: &[u8]) -> std::io::Result<Self> {
        let mut data = std::io::Cursor::new(data);

        let saved_capacity = ReadBytesExt::read_u64::<BigEndian>(&mut data)?;
        let saved_rtt = ReadBytesExt::read_u128::<BigEndian>(&mut data)?;

        let ip_ver = data.read_u8()?;
        let ip = match ip_ver {
            0x04 => {
                let mut subnet = [0u8; 4];
                data.read_exact(&mut subnet[..3])?;
                std::net::IpAddr::V4(std::net::Ipv4Addr::from(subnet))
            }
            0x06 => {
                let mut subnet = [0u8; 16];
                data.read_exact(&mut subnet[..6])?;
                std::net::IpAddr::V6(std::net::Ipv6Addr::from(subnet))
            }
            _ => return Err(std::io::ErrorKind::InvalidData.into())
        };

        let expiry_timestamp = ReadBytesExt::read_i64::<BigEndian>(&mut data)?;

        let mut signature = vec![0u8; HmacSha256::output_size()];
        std::io::Read::read_exact(&mut data, &mut signature)?;

        let saved_rtt = std::time::Duration::new(
            (saved_rtt / MICROS_PER_SEC) as u64, ((saved_rtt % MICROS_PER_SEC) as u32) * NANOS_PER_MICRO
        );
        let expiry = chrono::DateTime::from_timestamp(expiry_timestamp, 0)
            .ok_or_else(|| std::io::ErrorKind::InvalidData)?;

        Ok(Self {
            saved_capacity,
            saved_rtt,
            ip,
            expiry,
            signature
        })
    }

    pub fn verify_signature(&self, key: &[u8]) -> bool {
        if self.signature.len() != HmacSha256::output_size() {
            return false;
        }

        let signature_array = generic_array::GenericArray::<
            u8, <HmacSha256 as OutputSizeUser>::OutputSize
        >::from_slice(&self.signature);

        // Convert to constant time arrays for comparison
        let calculated_signature = self.make_mac(key);
        let stored_signature = digest::CtOutput::from(signature_array);

        calculated_signature == stored_signature
    }

    fn make_mac(&self, key: &[u8]) -> digest::CtOutput<HmacSha256> {
        use hmac::Mac;
        let mut mac = HmacSha256::new_from_slice(key).unwrap();

        let expiry_timestamp = self.expiry.timestamp();

        mac.update(&self.saved_capacity.to_be_bytes());
        mac.update(&self.saved_rtt.as_micros().to_be_bytes());
        mac.update(&expiry_timestamp.to_be_bytes());

       mac.finalize()
    }
}