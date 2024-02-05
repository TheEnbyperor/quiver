use tokio::io::AsyncWriteExt;

pub struct QLog {
    bytes_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl QLog {
    pub async fn new(path: impl AsRef<std::path::Path>) -> std::io::Result<(Self, tokio::task::JoinHandle<()>)> {
        let mut file = tokio::fs::File::create(path).await?;
        let (bytes_tx, mut bytes_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        let h = tokio::task::spawn(async move {
            while let Some(b) = bytes_rx.recv().await {
                if let Err(e) = file.write_all(&b).await {
                    warn!("Error writing qlog: {}", e);
                    break;
                }
            }
        });

        Ok((QLog { bytes_tx }, h))
    }
}

impl std::io::Write for QLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.bytes_tx.send(buf.to_vec()).is_err() {
            return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, ""));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}