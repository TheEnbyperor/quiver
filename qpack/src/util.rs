pub async fn encode_integer_async<W: tokio::io::AsyncWrite + Send + Sync + Unpin>(
    buf: &mut tokio_bitstream_io::write::BitWriter<W, tokio_bitstream_io::BigEndian>,
    n: u32,
    mut val: u64,
) -> std::io::Result<()> {
    use tokio_bitstream_io::BitWrite;
    let c = 2u64.pow(n) - 1u64;
    if val < c {
        buf.write(n, val).await?;
    } else {
        buf.write(n, c).await?;
        val -= c;
        while val >= 128 {
            buf.write(8, val % 128 + 128).await?;
            val /= 128;
        }
        buf.write(8, val).await?;
    }
    Ok(())
}

pub async fn decode_integer_async<R: tokio::io::AsyncRead + Send + Sync + Unpin>(
    buf: &mut tokio_bitstream_io::read::BitReader<R, tokio_bitstream_io::BigEndian>,
    n: u32,
) -> std::io::Result<u64> {
    use tokio_bitstream_io::BitRead;
    let mut i: u64 = buf.read(n).await?;
    let c = 2u64.pow(n) - 1u64;
    if i < c {
        return Ok(i);
    }
    let mut m = 0u32;
    loop {
        let b: u8 = buf.read(8).await?;
        i += (b as u64 & 127u64) * 2u64.pow(m);
        m += 7;
        if b & 128 != 128 {
            break;
        }
    }
    Ok(i)
}
