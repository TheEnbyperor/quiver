use bitstream_io::{BitRead, BitWrite};

enum Node {
    Terminal(u16),
    Node(&'static Node, &'static Node),
}

include!(concat!(env!("OUT_DIR"), "/qpack_huffman.rs"));

pub fn encode(data: &[u8]) -> Vec<u8> {
    let mut buf = bitstream_io::write::BitWriter::<_, bitstream_io::BigEndian>::new(
        std::io::Cursor::new(Vec::new()),
    );
    for byte in data {
        let code = HUFFMAN_TABLE[*byte as usize];
        buf.write(code.0 as u32, code.1).unwrap();
    }
    while !buf.byte_aligned() {
        buf.write_bit(true).unwrap();
    }
    buf.into_writer().into_inner()
}

pub fn decode(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut buf = bitstream_io::read::BitReader::<_, bitstream_io::BigEndian>::new(
        std::io::Cursor::new(data),
    );
    let mut out = vec![];
    let mut cur_node = HUFFMAN_TREE;
    let mut start = true;
    let mut valid_eos = true;
    loop {
        let (left, right) = match cur_node {
            Node::Node(l, r) => (l, r),
            Node::Terminal(t) => {
                if *t == 256 {
                    break;
                }
                out.push(*t as u8);
                cur_node = HUFFMAN_TREE;
                start = true;
                valid_eos = true;
                continue;
            }
        };
        let bit = match buf.read_bit() {
            Ok(b) => b,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::UnexpectedEof && (start || valid_eos) {
                    break;
                }
                return Err(err);
            }
        };
        match bit {
            false => {
                valid_eos = false;
                cur_node = left
            }
            true => cur_node = right,
        }
        start = false;
    }
    Ok(out)
}

#[test]
fn test_encode() {
    let test = b"test string";
    let encoded = encode(test);
    let decoded = decode(&encoded).unwrap();
    assert_eq!(test, &decoded[..]);
}
