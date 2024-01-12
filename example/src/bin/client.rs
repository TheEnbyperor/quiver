

#[macro_use]
extern crate log;

use clap::Parser;

const MAX_DATAGRAM_SIZE: usize = 1350;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    peer_addr: std::net::SocketAddr,
    #[arg(short, long)]
    local_addr: Option<std::net::SocketAddr>,
    #[arg(default_value = "/")]
    path: String,
}

async fn send_request(
    cid: &str, peer_addr: std::net::SocketAddr, local_addr: Option<std::net::SocketAddr>,
    path: &str, bdp_token: Option<quiver_bdp_tokens::BDPToken>
) -> Option<quiver_bdp_tokens::BDPToken> {
    let url = url::Url::parse("https://localhost/").unwrap();
    let url_host = url.host_str().unwrap();
    // let url_port = url.port_or_known_default().unwrap();
    // let url_authority = format!("{}:{}", url_host, url_port);
    let url_domain = url.domain().unwrap();
    // let peer_addrs = tokio::net::lookup_host(url_authority)
    //     .await
    //     .unwrap()
    //     .collect::<Vec<_>>();
    // let mut rng = thread_rng();
    // let peer_addr = *peer_addrs.choose(&mut rng).unwrap();
    // drop(rng);

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();
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
    config.enable_resume(false);

    let qlog = quiche_tokio::QLog::new(format!("./connection-{}.qlog", cid)).await.unwrap();
    let qlog_conf = quiche_tokio::QLogConfig {
        qlog,
        title: url.to_string(),
        description: String::new(),
        level: quiche::QlogLevel::Extra,
    };

    let bdp_token_bytes = match bdp_token {
        Some(t) => {
            let mut bdp_token_buf = std::io::Cursor::new(Vec::new());
            t.encode(&mut bdp_token_buf).unwrap();
            Some(bdp_token_buf.into_inner())
        }
        None => None
    };

    info!("Setting up QUIC connection to {} - {}", url, peer_addr);
    let mut connection =
        quiche_tokio::Connection::connect(
            peer_addr, config, Some(url_domain), local_addr, bdp_token_bytes.as_deref(), Some(qlog_conf)
        ).await.unwrap();
    connection.established().await.unwrap();
    info!("QUIC connection open");

    let trans_params = connection.transport_parameters().await.unwrap();
    let server_bdp_tokens = trans_params.bdp_tokens;
    let last_bdp_token = std::sync::Arc::new(tokio::sync::Mutex::new(None));

    if !server_bdp_tokens {
        warn!("Server not using BDP tokens");
    } else {
        info!("Server using BDP tokens");
        let mut new_token_recv = connection.new_tokens();
        let new_token_mtx = last_bdp_token.clone();
        tokio::task::spawn(async move {
            while let Some(token) = match new_token_recv.next().await {
                Ok(r) => r,
                Err(err) => {
                    // H3_NO_ERROR
                    if err.to_id() == 0x100 {
                        return
                    }
                    panic!("Error receiving tokens: {:?}", err);
                }
            } {
                trace!("New token received: {:02x?}", token);
                let mut bdp_token_buf = std::io::Cursor::new(token);
                let bdp_token = quiver_bdp_tokens::BDPToken::decode(&mut bdp_token_buf).unwrap();
                info!("BDP Token: {:?}", bdp_token);
                new_token_mtx.lock().await.replace(bdp_token);
            }
        });
    }

    let mut h3_connection = quiver_h3::Connection::new(connection, false);
    h3_connection.setup().await.unwrap();
    info!("HTTP/3 connection open");

    let mut headers = quiver_h3::Headers::new();
    headers.add(b":method", b"GET");
    headers.add(b":scheme", b"https");
    headers.add(b":authority", url_host.as_bytes());
    headers.add(b":path", path.as_bytes());
    headers.add(b"user-agent", b"quiche-tokio");

    info!("Sending request: {:#?}", headers);
    let mut response = h3_connection.send_request(&headers).await.unwrap();
    info!("Got response: {:#?}", response);

    let download_progress = indicatif::ProgressBar::new_spinner();

    let mut total = 0;
    while let Some(data) = response.get_next_data().await.unwrap() {
        total += data.len();
        download_progress.set_message(format!("Downloaded {}", indicatif::HumanBytes(total as u64)))
    }

    download_progress.finish();
    info!("Receive done");

    h3_connection.close().await.unwrap();
    info!("HTTP/3 and QUIC connection closed");

    let out = last_bdp_token.lock().await.take();
    out
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let args = Args::parse();

    let bdp_token = send_request(
        "1", args.peer_addr, args.local_addr, &args.path, None
    ).await;

    if let Some(bdp_token) = bdp_token {
        info!("Retrying connection with BDP token");
        send_request(
            "2", args.peer_addr, args.local_addr, &args.path, Some(bdp_token)
        ).await;
    } else {
        warn!("Didn't receive a BDP token from the server");
    }
}
