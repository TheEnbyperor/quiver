#[macro_use]
extern crate log;

use std::ops::DerefMut;
use clap::Parser;
use tokio::io::AsyncReadExt;

const MAX_DATAGRAM_SIZE: usize = 1350;

const FILE_PATH_1_MB: &'static str = "./1MB.bin";
const FILE_PATH_10_MB: &'static str = "./10MB.bin";
const FILE_PATH_100_MB: &'static str = "./100MB.bin";
const FILE_PATH_1_GB: &'static str = "./1GB.bin";

const BDP_KEY: &'static [u8] = b"test bdp key";

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    bind_addr: std::net::SocketAddr,
    #[arg(long, default_value_t = false)]
    pacing: bool,
}

struct CRCache {
    data: std::collections::HashMap<std::net::IpAddr, CRCacheEntry>
}

impl CRCache {
    fn new() -> Self {
        Self {
            data: std::collections::HashMap::new()
        }
    }

    fn add(&mut self, addr: std::net::IpAddr, lifetime: chrono::Duration, rtt: std::time::Duration, capacity: usize) {
        self.data.insert(addr, CRCacheEntry {
            expiry: chrono::Utc::now() + lifetime,
            rtt,
            capacity
        });
    }

    fn get(&mut self, addr: std::net::IpAddr) -> Option<CRCacheEntry> {
        let entry = self.data.remove(&addr)?;
        if entry.expired() {
            None
        } else {
            Some(entry)
        }
    }
}

struct CRCacheEntry {
    expiry: chrono::DateTime<chrono::Utc>,
    rtt: std::time::Duration,
    capacity: usize
}

impl CRCacheEntry {
    fn expired(&self) -> bool {
        self.expiry < chrono::Utc::now()
    }
}

enum CRState {
    Error,
    Disable,
    ClientCached {
        rtt: std::time::Duration,
        capacity: u64
    },
    ServerCached
}

fn common_subnet(lhs: std::net::IpAddr, rhs: std::net::IpAddr) -> bool {
    match (lhs, rhs) {
        (std::net::IpAddr::V4(lhs), std::net::IpAddr::V4(rhs)) => {
            &lhs.octets()[..3] == &rhs.octets()[..3]
        }
        (std::net::IpAddr::V6(lhs), std::net::IpAddr::V6(rhs)) => {
            &lhs.octets()[..6] == &rhs.octets()[..6]
        }
        _ => false
    }
}

async fn get_cr_state(connection: &quiche_tokio::Connection) -> CRState {
    let peer_token_bytes = connection.peer_token().await;
    if let Some(peer_token_bytes) = peer_token_bytes {
        info!("Received token from peer: {:02x?}", peer_token_bytes);
        let mut peer_token_buf = std::io::Cursor::new(peer_token_bytes);
        if let Ok(peer_token) = quiver_bdp_tokens::BDPToken::decode(&mut peer_token_buf) {
            info!("Decoded BDP token from peer: {:02x?}", peer_token);
            if peer_token.requested_capacity == 0 {
                info!("Client has requested careful resume be disabled");
                CRState::Disable
            } else if peer_token.bdp_data.len() == 0 {
                CRState::ServerCached
            } else if let Ok(peer_bdp_token) = quiver_bdp_tokens::CRBDPData::from_bytes(&peer_token.bdp_data) {
                info!("Decoded inner BDP token from peer: {:?}", peer_bdp_token);
                if peer_bdp_token.expired() {
                    warn!("Peer BDP token expired");
                    CRState::ServerCached
                } else if !peer_bdp_token.verify_signature(BDP_KEY) {
                    warn!("Failed to verify signature over BDP token");
                    CRState::ServerCached
                } else if !common_subnet(peer_bdp_token.ip(), connection.peer_addr().ip()) {
                    info!("Client on a different subnet, not using careful resume");
                    CRState::Disable
                } else {
                    info!("BDP token verified, using for careful resume");
                    let capacity = std::cmp::min(
                        peer_bdp_token.saved_capacity(), peer_token.requested_capacity
                    );
                    CRState::ClientCached {
                        rtt: peer_bdp_token.saved_rtt(),
                        capacity
                    }
                }
            } else {
                CRState::Error
            }
        } else {
            CRState::Error
        }
    } else {
        CRState::ServerCached
    }
}

async fn setup_cr(connection: &quiche_tokio::Connection, cr_cache: &mut CRCache) -> Result<(), quiche_tokio::ConnectionError> {
    let cr_state = get_cr_state(connection).await;
    match cr_state {
        CRState::Disable => Ok(()),
        CRState::Error => {
            connection.close(false, 0x1312, vec![]).await?;
            Ok(())
        }
        CRState::ClientCached { rtt, capacity } => {
            connection.setup_careful_resume(rtt, capacity as usize).await?;
            Ok(())
        }
        CRState::ServerCached => {
            if let Some(cr_entry) = cr_cache.get(connection.peer_addr().ip()) {
                connection.setup_careful_resume(cr_entry.rtt, cr_entry.capacity).await?;
            }
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let args = Args::parse();

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();
    config.load_cert_chain_from_pem_file("./cert.crt").unwrap();
    config.load_priv_key_from_pem_file("./cert.key").unwrap();
    config.verify_peer(false);
    config.set_application_protos(&[b"h3"]).unwrap();
    config.set_max_idle_timeout(5000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_stream_data_uni(1_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(true);
    config.set_bdp_tokens(true);
    config.enable_pacing(args.pacing);
    config.enable_resume(true);

    info!("Accepting QUIC connections on {}", args.bind_addr);
    let mut connections =
        quiche_tokio::Connection::accept(args.bind_addr, config)
            .await
            .unwrap();

    let cr_cache = std::sync::Arc::new(tokio::sync::Mutex::new(CRCache::new()));

    while let Some(mut connection) = connections.next().await {
        let cr_cache = cr_cache.clone();
        tokio::task::spawn(async move {
            info!("New connection");

            let scid = connection.scid();
            let qlog = quiche_tokio::QLog::new(format!("./connection-{:?}.qlog", scid)).await.unwrap();
            let qlog_conf = quiche_tokio::QLogConfig {
                qlog,
                title: format!("{:?}", scid),
                description: String::new(),
                level: quiche::QlogLevel::Extra,
            };
            connection.set_qlog(qlog_conf).await.unwrap();

            connection.established().await.unwrap();
            setup_cr(&connection, cr_cache.lock().await.deref_mut()).await.unwrap();

            let alpn = connection.application_protocol().await;
            info!("New connection established, alpn={}", String::from_utf8_lossy(&alpn));

            if alpn != b"h3" {
                warn!("Non HTTP/3 connection negotiated");
                return;
            }

            let mut cr_event_recv = connection.cr_events();
            let cr_event_connection = connection.send_half();
            tokio::task::spawn(async move {
                while let Some(cr_event) = cr_event_recv.next().await {
                    info!("New CR event: {:?}", cr_event);

                    let peer_ip = cr_event_connection.peer_addr().ip();
                    cr_cache.lock().await.add(
                        peer_ip, chrono::Duration::minutes(5), cr_event.min_rtt, cr_event.cwnd
                    );
                    let bdp_data = quiver_bdp_tokens::CRBDPData::from_quiche_cr_event(
                        &cr_event, peer_ip, chrono::Duration::minutes(5), BDP_KEY
                    );

                    let mut bdp_token = quiver_bdp_tokens::BDPToken::default();
                    bdp_token.saved_capacity = cr_event.cwnd as u64;
                    bdp_token.saved_rtt = cr_event.min_rtt.as_micros();
                    bdp_token.bdp_data = bdp_data.to_bytes();

                    let mut bdp_token_buf = std::io::Cursor::new(vec![]);
                    bdp_token.encode(&mut bdp_token_buf).unwrap();

                    cr_event_connection.send_new_token(bdp_token_buf.into_inner()).await.unwrap();
                }
            });

            let mut h3_connection = quiver_h3::Connection::new(connection, true);
            h3_connection.setup().await.unwrap();
            info!("HTTP/3 connection open");

            while let Some(mut request) = h3_connection.next_request().await.unwrap() {
                tokio::task::spawn(async move {
                    info!("New request");

                    let headers = request.headers();
                    match headers.get_header_one(b":method") {
                        Some(m) => {
                            if m.value.as_ref() != b"GET" {
                                let mut headers = quiver_h3::Headers::new();
                                headers.add(b":status", b"405");
                                request.send_headers(&headers).await.unwrap();
                                request.done().await.unwrap();
                                return;
                            }
                        },
                        None => {
                            let mut headers = quiver_h3::Headers::new();
                            headers.add(b":status", b"400");
                            request.send_headers(&headers).await.unwrap();
                            request.done().await.unwrap();
                            return;
                        }
                    };
                    let scheme = headers.get_header_one(b":scheme");
                    let path = headers.get_header_one(b":path");

                    if scheme.is_none() || path.is_none() {
                        let mut headers = quiver_h3::Headers::new();
                        headers.add(b":status", b"400");
                        request.send_headers(&headers).await.unwrap();
                        request.done().await.unwrap();
                        return;
                    }

                    let scheme = scheme.unwrap();
                    let path = path.unwrap();

                    if scheme.value.as_ref() != b"https" {
                        let mut headers = quiver_h3::Headers::new();
                        headers.add(b":status", b"400");
                        request.send_headers(&headers).await.unwrap();
                        request.done().await.unwrap();
                        return;
                    }

                    match path.value.as_ref() {
                        b"/" => {
                            let mut headers = quiver_h3::Headers::new();
                            headers.add(b":status", b"200");
                            request.send_headers(&headers).await.unwrap();
                            request.send_data(b"hello world").await.unwrap();
                            request.done().await.unwrap();
                        }
                        b"/1MB.bin" => {
                            send_file(&mut request, FILE_PATH_1_MB).await;
                        }
                        b"/10MB.bin" => {
                            send_file(&mut request, FILE_PATH_10_MB).await;
                        }
                        b"/100MB.bin" => {
                            send_file(&mut request, FILE_PATH_100_MB).await;
                        }
                        b"/1GB.bin" => {
                            send_file(&mut request, FILE_PATH_1_GB).await;
                        }
                        _ => {
                            let mut headers = quiver_h3::Headers::new();
                            headers.add(b":status", b"400");
                            request.send_headers(&headers).await.unwrap();
                            request.done().await.unwrap();
                        }
                    }

                    info!("Request done");
                });
            }
            info!("HTTP/3 connection closed");
        });
    }
}

async fn send_file<S: AsRef<std::path::Path>>(request: &mut quiver_h3::Message, path: S) {
    let mut headers = quiver_h3::Headers::new();
    headers.add(b":status", b"200");
    request.send_headers(&headers).await.unwrap();
    let file = tokio::fs::File::open(path).await.unwrap();
    let mut buffer = [0; 4096];
    let mut reader = tokio::io::BufReader::new(file);
    loop {
        let n = reader.read(&mut buffer).await.unwrap();
        if n == 0 {
            break;
        }
        request.send_data(&buffer[..n]).await.unwrap();
    }
    request.done().await.unwrap();
}
