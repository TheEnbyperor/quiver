use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn read_int<R: tokio::io::AsyncRead + Unpin>(buf: &mut R) -> std::io::Result<u64> {
    let mut v = buf.read_u8().await? as u64;
    let length = 1usize << (v >> 6usize);

    v &= 0x3f;
    for _ in 0..length - 1 {
        v = (v << 8) + (buf.read_u8().await? as u64);
    }

    Ok(v)
}

pub async fn write_int<W: tokio::io::AsyncWrite + Unpin>(
    buf: &mut W,
    val: u64,
) -> std::io::Result<()> {
    if val <= 63 {
        buf.write_u8(val as u8).await?;
    } else if val <= 16383 {
        buf.write_u8(0b01000000 | (val >> 8) as u8).await?;
        buf.write_u8((val & 0xFF) as u8).await?;
    } else if val <= 1073741823 {
        buf.write_u8(0b10000000 | (val >> 24) as u8).await?;
        buf.write_u8(((val >> 16) & 0xFF) as u8).await?;
        buf.write_u8(((val >> 8) & 0xFF) as u8).await?;
        buf.write_u8((val & 0xFF) as u8).await?;
    } else {
        buf.write_u8(0b11000000 | (val >> 56) as u8).await?;
        buf.write_u8(((val >> 48) & 0xFF) as u8).await?;
        buf.write_u8(((val >> 40) & 0xFF) as u8).await?;
        buf.write_u8(((val >> 32) & 0xFF) as u8).await?;
        buf.write_u8(((val >> 24) & 0xFF) as u8).await?;
        buf.write_u8(((val >> 16) & 0xFF) as u8).await?;
        buf.write_u8(((val >> 8) & 0xFF) as u8).await?;
        buf.write_u8((val & 0xFF) as u8).await?;
    }
    Ok(())
}

#[tokio::test]
async fn test_eight_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("c2197c5eff14e88c").unwrap());
    assert_eq!(read_int(&mut data).await.unwrap(), 151_288_809_941_952_652);
}

#[tokio::test]
async fn test_eight_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 151_288_809_941_952_652).await.unwrap();
    assert_eq!(data.into_inner(), hex::decode("c2197c5eff14e88c").unwrap());
}

#[tokio::test]
async fn test_four_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("9d7f3e7d").unwrap());
    assert_eq!(read_int(&mut data).await.unwrap(), 494_878_333);
}

#[tokio::test]
async fn test_four_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 494_878_333).await.unwrap();
    assert_eq!(data.into_inner(), hex::decode("9d7f3e7d").unwrap());
}

#[tokio::test]
async fn test_two_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("7bbd").unwrap());
    assert_eq!(read_int(&mut data).await.unwrap(), 15_293);
}

#[tokio::test]
async fn test_two_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 15_293).await.unwrap();
    assert_eq!(data.into_inner(), hex::decode("7bbd").unwrap());
}

#[tokio::test]
async fn test_one_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("25").unwrap());
    assert_eq!(read_int(&mut data).await.unwrap(), 37);
}

#[tokio::test]
async fn test_one_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 37).await.unwrap();
    assert_eq!(data.into_inner(), hex::decode("25").unwrap());
}
