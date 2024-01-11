pub fn handle_http_io_error<T>(r: Result<T, std::io::Error>) -> super::error::HttpResult<Option<T>> {
    match r {
        Ok(t) => Ok(Some(t)),
        Err(err) => {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(None);
            }
            let is_quic_err = err.get_ref().and_then(|e| {
                e.downcast_ref::<quiche_tokio::ConnectionError>()
            }).is_some();
            if is_quic_err {
                let err = *err.into_inner().unwrap()
                    .downcast::<quiche_tokio::ConnectionError>().unwrap();
                // H3_NO_ERROR
                if err.to_id() == 0x100 {
                    Ok(None)
                } else {
                    Err(err.into())
                }
            } else {
                Err(err.into())
            }
        }
    }
}