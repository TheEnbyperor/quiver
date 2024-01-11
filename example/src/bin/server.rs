#[macro_use]
extern crate log;

const MAX_DATAGRAM_SIZE: usize = 1350;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

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

    let bind_addr = "[::]:4443".parse().unwrap();

    info!("Accepting QUIC connections on {}", bind_addr);
    let mut connections =
        quiche_tokio::Connection::accept(bind_addr, config)
            .await
            .unwrap();

    while let Some(connection) = connections.next().await {
        tokio::task::spawn(async move {
            info!("New connection");
            connection.established().await.unwrap();
            let alpn = connection.application_protocol().await;
            info!("New connection established, alpn={}", String::from_utf8_lossy(&alpn));

            if alpn != b"h3" {
                warn!("Non HTTP/3 connection negotiated");
                return;
            }

            let mut h3_connection = quiver_h3::Connection::new(connection, true);
            h3_connection.setup().await.unwrap();
            info!("HTTP/3 connection open");

            let mut test_bdp_token = quiver_bdp_tokens::BDPToken::default();
            test_bdp_token.saved_capacity = 1320;
            test_bdp_token.saved_rtt = 12500;
            let mut test_bdp_token_buf = std::io::Cursor::new(vec![]);
            test_bdp_token.encode(&mut test_bdp_token_buf).await.unwrap();

            h3_connection.inner_connection().send_new_token(test_bdp_token_buf.into_inner()).await.unwrap();

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