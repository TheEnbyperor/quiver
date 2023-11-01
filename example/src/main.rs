#![feature(extract_if)]

#[macro_use]
extern crate log;

use rand::prelude::*;

const MAX_DATAGRAM_SIZE: usize = 1350;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let url = url::Url::parse("https://cloudflare-quic.com/").unwrap();
    let url_host = url.host_str().unwrap();
    let url_port = url.port_or_known_default().unwrap();
    let url_authority = format!("{}:{}", url_host, url_port);
    let url_domain = url.domain().unwrap();
    let peer_addrs = tokio::net::lookup_host(url_authority)
        .await
        .unwrap()
        .collect::<Vec<_>>();
    let mut rng = thread_rng();
    let peer_addr = *peer_addrs.choose(&mut rng).unwrap();
    drop(rng);

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();
    config.verify_peer(true);
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

    let qlog = quiche_tokio::QLog::new("./connection.qlog").await.unwrap();
    let qlog_conf = quiche_tokio::QLogConfig {
        qlog,
        title: url.to_string(),
        description: String::new(),
        level: quiche::QlogLevel::Extra,
    };

    info!("Setting up QUIC connection to {}", url);
    let connection =
        quiche_tokio::Connection::connect(peer_addr, config, Some(url_domain), Some(qlog_conf))
            .await
            .unwrap();
    connection.established().await.unwrap();
    info!("QUIC connection open");

    let mut h3_connection = quiver_h3::Connection::new(connection);
    h3_connection.setup().await.unwrap();
    info!("HTTP/3 connection open");

    let mut headers = quiver_h3::Headers::new();
    headers.add(b":method", b"GET");
    headers.add(b":scheme", b"https");
    headers.add(b":authority", url_host.as_bytes());
    headers.add(b":path", b"/");
    headers.add(b"user-agent", b"quiche-tokio");

    info!("Sending request: {:#?}", headers);
    let mut response = h3_connection.send_request(&headers).await.unwrap();
    info!("Got response: {:#?}", response);
    let response_data = response.data().await.unwrap();
    println!("{}", String::from_utf8(response_data).unwrap());

    h3_connection.close().await.unwrap();
    info!("HTTP/3 and QUIC connection closed");
}
