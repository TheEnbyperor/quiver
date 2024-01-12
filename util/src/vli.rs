pub trait Int: num_traits::Unsigned + num_traits::PrimInt + num_traits::AsPrimitive<usize> + num_traits::AsPrimitive<u8>
    + num_traits::FromPrimitive + std::ops::Shl<usize> + std::ops::BitAndAssign<Self> + std::ops::BitAnd<Self> {}

impl Int for u8 {}
impl Int for u16 {}
impl Int for u32 {}
impl Int for u64 {}
impl Int for u128 {}
impl Int for usize {}

pub fn read_int<N: Int, R: std::io::Read>(buf: &mut R) -> std::io::Result<N> {
    use byteorder::ReadBytesExt;

    let mut v: N = N::from_u8(buf.read_u8()?).unwrap();
    let length = 1usize << (<N as num_traits::AsPrimitive<usize>>::as_(v) >> 6usize);

    v &= N::from_u8(0x3f).unwrap();
    for _ in 0..length - 1 {
        v = v.unsigned_shl(8) + N::from_u8(buf.read_u8()?).unwrap();
    }

    Ok(v)
}

pub async fn read_int_async<N: Int, R: tokio::io::AsyncRead + Unpin>(buf: &mut R) -> std::io::Result<N> {
    use tokio::io::AsyncReadExt;

    let mut v: N = N::from_u8(buf.read_u8().await?).unwrap();
    let length = 1usize << (<N as num_traits::AsPrimitive<usize>>::as_(v) >> 6usize);

    v &= N::from_u8(0x3f).unwrap();
    for _ in 0..length - 1 {
        v = v.unsigned_shl(8) + N::from_u8(buf.read_u8().await?).unwrap();
    }

    Ok(v)
}

pub fn write_int<N: Int, W: std::io::Write>(
    buf: &mut W,
    val: N,
) -> std::io::Result<()> {
    use byteorder::WriteBytesExt;

    let max_byte = N::from_u8(0xFF).unwrap();
    if val <= N::from(63).unwrap() {
        buf.write_u8(val.as_())?;
    } else if val <= N::from(16383).unwrap() {
        buf.write_u8(0b01000000 | <N as num_traits::AsPrimitive<u8>>::as_(val >> 8))?;
        buf.write_u8((val & max_byte).as_())?;
    } else if val <= N::from(1073741823).unwrap() {
        buf.write_u8(0b10000000 | <N as num_traits::AsPrimitive<u8>>::as_(val >> 24))?;
        buf.write_u8(((val >> 16) & max_byte).as_())?;
        buf.write_u8(((val >> 8) & max_byte).as_())?;
        buf.write_u8((val & max_byte).as_())?;
    } else {
        buf.write_u8(0b11000000 | <N as num_traits::AsPrimitive<u8>>::as_(val >> 56))?;
        buf.write_u8(((val >> 48) & max_byte).as_())?;
        buf.write_u8(((val >> 40) & max_byte).as_())?;
        buf.write_u8(((val >> 32) & max_byte).as_())?;
        buf.write_u8(((val >> 24) & max_byte).as_())?;
        buf.write_u8(((val >> 16) & max_byte).as_())?;
        buf.write_u8(((val >> 8) & max_byte).as_())?;
        buf.write_u8((val & max_byte).as_())?;
    }
    Ok(())
}

pub async fn write_int_async<N: Int, W: tokio::io::AsyncWrite + Unpin>(
    buf: &mut W,
    val: N,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let max_byte = N::from_u8(0xFF).unwrap();
    if val <= N::from(63).unwrap() {
        buf.write_u8(val.as_()).await?;
    } else if val <= N::from(16383).unwrap() {
        buf.write_u8(0b01000000 | <N as num_traits::AsPrimitive<u8>>::as_(val >> 8)).await?;
        buf.write_u8((val & max_byte).as_()).await?;
    } else if val <= N::from(1073741823).unwrap() {
        buf.write_u8(0b10000000 | <N as num_traits::AsPrimitive<u8>>::as_(val >> 24)).await?;
        buf.write_u8(((val >> 16) & max_byte).as_()).await?;
        buf.write_u8(((val >> 8) & max_byte).as_()).await?;
        buf.write_u8((val & max_byte).as_()).await?;
    } else {
        buf.write_u8(0b11000000 | <N as num_traits::AsPrimitive<u8>>::as_(val >> 56)).await?;
        buf.write_u8(((val >> 48) & max_byte).as_()).await?;
        buf.write_u8(((val >> 40) & max_byte).as_()).await?;
        buf.write_u8(((val >> 32) & max_byte).as_()).await?;
        buf.write_u8(((val >> 24) & max_byte).as_()).await?;
        buf.write_u8(((val >> 16) & max_byte).as_()).await?;
        buf.write_u8(((val >> 8) & max_byte).as_()).await?;
        buf.write_u8((val & max_byte).as_()).await?;
    }
    Ok(())
}

#[test]
fn test_eight_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("c2197c5eff14e88c").unwrap());
    assert_eq!(read_int::<u64, _>(&mut data).unwrap(), 151_288_809_941_952_652u64);
}

#[test]
fn test_eight_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 151_288_809_941_952_652u64).unwrap();
    assert_eq!(data.into_inner(), hex::decode("c2197c5eff14e88c").unwrap());
}

#[test]
fn test_four_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("9d7f3e7d").unwrap());
    assert_eq!(read_int::<u32, _>(&mut data).unwrap(), 494_878_333u32);
}

#[test]
fn test_four_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 494_878_333u32).unwrap();
    assert_eq!(data.into_inner(), hex::decode("9d7f3e7d").unwrap());
}

#[test]
fn test_two_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("7bbd").unwrap());
    assert_eq!(read_int::<u16, _>(&mut data).unwrap(), 15_293u16);
}

#[test]
fn test_two_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 15_293u16).unwrap();
    assert_eq!(data.into_inner(), hex::decode("7bbd").unwrap());
}

#[test]
fn test_one_byte_dec() {
    let mut data = std::io::Cursor::new(hex::decode("25").unwrap());
    assert_eq!(read_int::<u8, _>(&mut data).unwrap(), 37u8);
}

#[test]
fn test_one_byte_enc() {
    let mut data = std::io::Cursor::new(Vec::new());
    write_int(&mut data, 37u8).unwrap();
    assert_eq!(data.into_inner(), hex::decode("25").unwrap());
}
