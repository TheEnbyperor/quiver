use std::io::Write;

#[derive(Eq, PartialEq, Hash, Debug)]
struct TableEntry {
    name: &'static [u8],
    value: &'static [u8],
}

impl TableEntry {
    const fn new(name: &'static [u8], value: &'static [u8]) -> Self {
        Self { name, value }
    }
}

impl phf::PhfHash for TableEntry {
    #[inline]
    fn phf_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.phf_hash(state);
        self.value.phf_hash(state);
    }
}

impl phf_shared::FmtConst for TableEntry {
    fn fmt_const(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TableEntry {{name: &{:?}, value: &{:?}}}",
            self.name, self.value
        )
    }
}

const STATIC_TABLE: &[TableEntry] = &[
    TableEntry::new(b":authority", b""),
    TableEntry::new(b":path", b"/"),
    TableEntry::new(b"age", b"0"),
    TableEntry::new(b"content-disposition", b""),
    TableEntry::new(b"content-length", b"0"),
    TableEntry::new(b"cookie", b""),
    TableEntry::new(b"date", b""),
    TableEntry::new(b"etag", b""),
    TableEntry::new(b"if-modified-since", b""),
    TableEntry::new(b"if-none-match", b""),
    TableEntry::new(b"last-modified", b""),
    TableEntry::new(b"link", b""),
    TableEntry::new(b"location", b""),
    TableEntry::new(b"referer", b""),
    TableEntry::new(b"set-cookie", b""),
    TableEntry::new(b":method", b"CONNECT"),
    TableEntry::new(b":method", b"DELETE"),
    TableEntry::new(b":method", b"GET"),
    TableEntry::new(b":method", b"HEAD"),
    TableEntry::new(b":method", b"OPTIONS"),
    TableEntry::new(b":method", b"POST"),
    TableEntry::new(b":method", b"PUT"),
    TableEntry::new(b":scheme", b"http"),
    TableEntry::new(b":scheme", b"https"),
    TableEntry::new(b":status", b"103"),
    TableEntry::new(b":status", b"200"),
    TableEntry::new(b":status", b"304"),
    TableEntry::new(b":status", b"404"),
    TableEntry::new(b":status", b"503"),
    TableEntry::new(b"accept", b"*/*"),
    TableEntry::new(b"accept", b"application/dns-message"),
    TableEntry::new(b"accept-encoding", b"gzip, deflate, br"),
    TableEntry::new(b"accept-ranges", b"bytes"),
    TableEntry::new(b"access-control-allow-headers", b"cache-control"),
    TableEntry::new(b"access-control-allow-headers", b"content-type"),
    TableEntry::new(b"access-control-allow-origin", b"*"),
    TableEntry::new(b"cache-control", b"max-age=0"),
    TableEntry::new(b"cache-control", b"max-age=2592000"),
    TableEntry::new(b"cache-control", b"max-age=604800"),
    TableEntry::new(b"cache-control", b"no-cache"),
    TableEntry::new(b"cache-control", b"no-store"),
    TableEntry::new(b"cache-control", b"public, max-age=31536000"),
    TableEntry::new(b"content-encoding", b"br"),
    TableEntry::new(b"content-encoding", b"gzip"),
    TableEntry::new(b"content-type", b"application/dns-message"),
    TableEntry::new(b"content-type", b"application/javascript"),
    TableEntry::new(b"content-type", b"application/json"),
    TableEntry::new(b"content-type", b"application/x-www-form-urlencoded"),
    TableEntry::new(b"content-type", b"image/gif"),
    TableEntry::new(b"content-type", b"image/jpeg"),
    TableEntry::new(b"content-type", b"image/png"),
    TableEntry::new(b"content-type", b"text/css"),
    TableEntry::new(b"content-type", b"text/html; charset=utf-8"),
    TableEntry::new(b"content-type", b"text/plain"),
    TableEntry::new(b"content-type", b"text/plain;charset=utf-8"),
    TableEntry::new(b"range", b"bytes=0-"),
    TableEntry::new(b"strict-transport-security", b"max-age=31536000"),
    TableEntry::new(
        b"strict-transport-security",
        b"max-age=31536000; includesubdomains",
    ),
    TableEntry::new(
        b"strict-transport-security",
        b"max-age=31536000; includesubdomains; preload",
    ),
    TableEntry::new(b"vary", b"accept-encoding"),
    TableEntry::new(b"vary", b"origin"),
    TableEntry::new(b"x-content-type-options", b"nosniff"),
    TableEntry::new(b"x-xss-protection", b"1; mode=block"),
    TableEntry::new(b":status", b"100"),
    TableEntry::new(b":status", b"204"),
    TableEntry::new(b":status", b"206"),
    TableEntry::new(b":status", b"302"),
    TableEntry::new(b":status", b"400"),
    TableEntry::new(b":status", b"403"),
    TableEntry::new(b":status", b"421"),
    TableEntry::new(b":status", b"425"),
    TableEntry::new(b":status", b"500"),
    TableEntry::new(b"accept-language", b""),
    TableEntry::new(b"access-control-allow-credentials", b"FALSE"),
    TableEntry::new(b"access-control-allow-credentials", b"TRUE"),
    TableEntry::new(b"access-control-allow-headers", b"*"),
    TableEntry::new(b"access-control-allow-methods", b"get"),
    TableEntry::new(b"access-control-allow-methods", b"get, post, options"),
    TableEntry::new(b"access-control-allow-methods", b"options"),
    TableEntry::new(b"access-control-expose-headers", b"content-length"),
    TableEntry::new(b"access-control-request-headers", b"content-type"),
    TableEntry::new(b"access-control-request-method", b"get"),
    TableEntry::new(b"access-control-request-method", b"post"),
    TableEntry::new(b"alt-svc", b"clear"),
    TableEntry::new(b"authorization", b""),
    TableEntry::new(
        b"content-security-policy",
        b"script-src 'none'; object-src 'none'; base-uri 'none'",
    ),
    TableEntry::new(b"early-data", b"1"),
    TableEntry::new(b"expect-ct", b""),
    TableEntry::new(b"forwarded", b""),
    TableEntry::new(b"if-range", b""),
    TableEntry::new(b"origin", b""),
    TableEntry::new(b"purpose", b"prefetch"),
    TableEntry::new(b"server", b""),
    TableEntry::new(b"timing-allow-origin", b"*"),
    TableEntry::new(b"upgrade-insecure-requests", b"1"),
    TableEntry::new(b"user-agent", b""),
    TableEntry::new(b"x-forwarded-for", b""),
    TableEntry::new(b"x-frame-options", b"deny"),
    TableEntry::new(b"x-frame-options", b"sameorigin"),
];

const HUFFMAN_TABLE: &[(u32, u8, u8)] = &[
    (0x1ff8, 13, 0),
    (0x7fffd8, 23, 1),
    (0xfffffe2, 28, 2),
    (0xfffffe3, 28, 3),
    (0xfffffe4, 28, 4),
    (0xfffffe5, 28, 5),
    (0xfffffe6, 28, 6),
    (0xfffffe7, 28, 7),
    (0xfffffe8, 28, 8),
    (0xffffea, 24, 9),
    (0x3ffffffc, 30, 10),
    (0xfffffe9, 28, 11),
    (0xfffffea, 28, 12),
    (0x3ffffffd, 30, 13),
    (0xfffffeb, 28, 14),
    (0xfffffec, 28, 15),
    (0xfffffed, 28, 16),
    (0xfffffee, 28, 17),
    (0xfffffef, 28, 18),
    (0xffffff0, 28, 19),
    (0xffffff1, 28, 20),
    (0xffffff2, 28, 21),
    (0x3ffffffe, 30, 22),
    (0xffffff3, 28, 23),
    (0xffffff4, 28, 24),
    (0xffffff5, 28, 25),
    (0xffffff6, 28, 26),
    (0xffffff7, 28, 27),
    (0xffffff8, 28, 28),
    (0xffffff9, 28, 29),
    (0xffffffa, 28, 30),
    (0xffffffb, 28, 31),
    (0x14, 6, 32),
    (0x3f8, 10, 33),
    (0x3f9, 10, 34),
    (0xffa, 12, 35),
    (0x1ff9, 13, 36),
    (0x15, 6, 37),
    (0xf8, 8, 38),
    (0x7fa, 11, 39),
    (0x3fa, 10, 40),
    (0x3fb, 10, 41),
    (0xf9, 8, 42),
    (0x7fb, 11, 43),
    (0xfa, 8, 44),
    (0x16, 6, 45),
    (0x17, 6, 46),
    (0x18, 6, 47),
    (0x0, 5, 48),
    (0x1, 5, 49),
    (0x2, 5, 50),
    (0x19, 6, 51),
    (0x1a, 6, 52),
    (0x1b, 6, 53),
    (0x1c, 6, 54),
    (0x1d, 6, 55),
    (0x1e, 6, 56),
    (0x1f, 6, 57),
    (0x5c, 7, 58),
    (0xfb, 8, 59),
    (0x7ffc, 15, 60),
    (0x20, 6, 61),
    (0xffb, 12, 62),
    (0x3fc, 10, 63),
    (0x1ffa, 13, 64),
    (0x21, 6, 65),
    (0x5d, 7, 66),
    (0x5e, 7, 67),
    (0x5f, 7, 68),
    (0x60, 7, 69),
    (0x61, 7, 70),
    (0x62, 7, 71),
    (0x63, 7, 72),
    (0x64, 7, 73),
    (0x65, 7, 74),
    (0x66, 7, 75),
    (0x67, 7, 76),
    (0x68, 7, 77),
    (0x69, 7, 78),
    (0x6a, 7, 79),
    (0x6b, 7, 80),
    (0x6c, 7, 81),
    (0x6d, 7, 82),
    (0x6e, 7, 83),
    (0x6f, 7, 84),
    (0x70, 7, 85),
    (0x71, 7, 86),
    (0x72, 7, 87),
    (0xfc, 8, 88),
    (0x73, 7, 89),
    (0xfd, 8, 90),
    (0x1ffb, 13, 91),
    (0x7fff0, 19, 92),
    (0x1ffc, 13, 93),
    (0x3ffc, 14, 94),
    (0x22, 6, 95),
    (0x7ffd, 15, 96),
    (0x3, 5, 97),
    (0x23, 6, 98),
    (0x4, 5, 99),
    (0x24, 6, 100),
    (0x5, 5, 101),
    (0x25, 6, 102),
    (0x26, 6, 103),
    (0x27, 6, 104),
    (0x6, 5, 105),
    (0x74, 7, 106),
    (0x75, 7, 107),
    (0x28, 6, 108),
    (0x29, 6, 109),
    (0x2a, 6, 110),
    (0x7, 5, 111),
    (0x2b, 6, 112),
    (0x76, 7, 113),
    (0x2c, 6, 114),
    (0x8, 5, 115),
    (0x9, 5, 116),
    (0x2d, 6, 117),
    (0x77, 7, 118),
    (0x78, 7, 119),
    (0x79, 7, 120),
    (0x7a, 7, 121),
    (0x7b, 7, 122),
    (0x7ffe, 15, 123),
    (0x7fc, 11, 124),
    (0x3ffd, 14, 125),
    (0x1ffd, 13, 126),
    (0xffffffc, 28, 127),
    (0xfffe6, 20, 128),
    (0x3fffd2, 22, 129),
    (0xfffe7, 20, 130),
    (0xfffe8, 20, 131),
    (0x3fffd3, 22, 132),
    (0x3fffd4, 22, 133),
    (0x3fffd5, 22, 134),
    (0x7fffd9, 23, 135),
    (0x3fffd6, 22, 136),
    (0x7fffda, 23, 137),
    (0x7fffdb, 23, 138),
    (0x7fffdc, 23, 139),
    (0x7fffdd, 23, 140),
    (0x7fffde, 23, 141),
    (0xffffeb, 24, 142),
    (0x7fffdf, 23, 143),
    (0xffffec, 24, 144),
    (0xffffed, 24, 145),
    (0x3fffd7, 22, 146),
    (0x7fffe0, 23, 147),
    (0xffffee, 24, 148),
    (0x7fffe1, 23, 149),
    (0x7fffe2, 23, 150),
    (0x7fffe3, 23, 151),
    (0x7fffe4, 23, 152),
    (0x1fffdc, 21, 153),
    (0x3fffd8, 22, 154),
    (0x7fffe5, 23, 155),
    (0x3fffd9, 22, 156),
    (0x7fffe6, 23, 157),
    (0x7fffe7, 23, 158),
    (0xffffef, 24, 159),
    (0x3fffda, 22, 160),
    (0x1fffdd, 21, 161),
    (0xfffe9, 20, 162),
    (0x3fffdb, 22, 163),
    (0x3fffdc, 22, 164),
    (0x7fffe8, 23, 165),
    (0x7fffe9, 23, 166),
    (0x1fffde, 21, 167),
    (0x7fffea, 23, 168),
    (0x3fffdd, 22, 169),
    (0x3fffde, 22, 170),
    (0xfffff0, 24, 171),
    (0x1fffdf, 21, 172),
    (0x3fffdf, 22, 173),
    (0x7fffeb, 23, 174),
    (0x7fffec, 23, 175),
    (0x1fffe0, 21, 176),
    (0x1fffe1, 21, 177),
    (0x3fffe0, 22, 178),
    (0x1fffe2, 21, 179),
    (0x7fffed, 23, 180),
    (0x3fffe1, 22, 181),
    (0x7fffee, 23, 182),
    (0x7fffef, 23, 183),
    (0xfffea, 20, 184),
    (0x3fffe2, 22, 185),
    (0x3fffe3, 22, 186),
    (0x3fffe4, 22, 187),
    (0x7ffff0, 23, 188),
    (0x3fffe5, 22, 189),
    (0x3fffe6, 22, 190),
    (0x7ffff1, 23, 191),
    (0x3ffffe0, 26, 192),
    (0x3ffffe1, 26, 193),
    (0xfffeb, 20, 194),
    (0x7fff1, 19, 195),
    (0x3fffe7, 22, 196),
    (0x7ffff2, 23, 197),
    (0x3fffe8, 22, 198),
    (0x1ffffec, 25, 199),
    (0x3ffffe2, 26, 200),
    (0x3ffffe3, 26, 201),
    (0x3ffffe4, 26, 202),
    (0x7ffffde, 27, 203),
    (0x7ffffdf, 27, 204),
    (0x3ffffe5, 26, 205),
    (0xfffff1, 24, 206),
    (0x1ffffed, 25, 207),
    (0x7fff2, 19, 208),
    (0x1fffe3, 21, 209),
    (0x3ffffe6, 26, 210),
    (0x7ffffe0, 27, 211),
    (0x7ffffe1, 27, 212),
    (0x3ffffe7, 26, 213),
    (0x7ffffe2, 27, 214),
    (0xfffff2, 24, 215),
    (0x1fffe4, 21, 216),
    (0x1fffe5, 21, 217),
    (0x3ffffe8, 26, 218),
    (0x3ffffe9, 26, 219),
    (0xffffffd, 28, 220),
    (0x7ffffe3, 27, 221),
    (0x7ffffe4, 27, 222),
    (0x7ffffe5, 27, 223),
    (0xfffec, 20, 224),
    (0xfffff3, 24, 225),
    (0xfffed, 20, 226),
    (0x1fffe6, 21, 227),
    (0x3fffe9, 22, 228),
    (0x1fffe7, 21, 229),
    (0x1fffe8, 21, 230),
    (0x7ffff3, 23, 231),
    (0x3fffea, 22, 232),
    (0x3fffeb, 22, 233),
    (0x1ffffee, 25, 234),
    (0x1ffffef, 25, 235),
    (0xfffff4, 24, 236),
    (0xfffff5, 24, 237),
    (0x3ffffea, 26, 238),
    (0x7ffff4, 23, 239),
    (0x3ffffeb, 26, 240),
    (0x7ffffe6, 27, 241),
    (0x3ffffec, 26, 242),
    (0x3ffffed, 26, 243),
    (0x7ffffe7, 27, 244),
    (0x7ffffe8, 27, 245),
    (0x7ffffe9, 27, 246),
    (0x7ffffea, 27, 247),
    (0x7ffffeb, 27, 248),
    (0xffffffe, 28, 249),
    (0x7ffffec, 27, 250),
    (0x7ffffed, 27, 251),
    (0x7ffffee, 27, 252),
    (0x7ffffef, 27, 253),
    (0x7fffff0, 27, 254),
    (0x3ffffee, 26, 255),
    (0x3fffffff, 30, 0),
];

fn main() {
    let path =
        std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("qpack_static_table.rs");
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());

    writeln!(
        &mut file,
        "const STATIC_TABLE: &'static [TableEntry] = &["
    )
    .unwrap();
    for entry in STATIC_TABLE {
        writeln!(
            &mut file,
            "TableEntry {{name: &{:?}, value: &{:?}}},",
            entry.name, entry.value
        )
        .unwrap();
    }
    writeln!(&mut file, "];").unwrap();

    let mut name_lookup = phf_codegen::Map::new();
    let mut seen_names = std::collections::HashSet::new();
    for (i, entry) in STATIC_TABLE.iter().enumerate() {
        if seen_names.contains(entry.name) {
            continue;
        }
        name_lookup.entry(entry.name, &format!("{}", i));
        seen_names.insert(entry.name);
    }
    writeln!(
        &mut file,
        "static NAME_LOOKUP: phf::Map<&'static [u8], usize> = {};",
        name_lookup.build()
    )
    .unwrap();

    let mut name_value_lookup = phf_codegen::Map::new();
    for (i, entry) in STATIC_TABLE.iter().enumerate() {
        name_value_lookup.entry(entry, &format!("{}", i));
    }
    writeln!(
        &mut file,
        "static NAME_VALUE_LOOKUP: phf::Map<TableEntry, usize> = {};",
        name_value_lookup.build()
    )
    .unwrap();

    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("qpack_huffman.rs");
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());

    write!(&mut file, "static HUFFMAN_TREE: &'static Node = ").unwrap();
    make_huffman_node(&mut file, 0, 1);
    writeln!(&mut file, ";").unwrap();

    write!(&mut file, "static HUFFMAN_TABLE: &'static[(u8, u32)] = &[").unwrap();
    let mut huffman_table = HUFFMAN_TABLE[..HUFFMAN_TABLE.len() - 1].to_vec();
    huffman_table.sort_by_key(|(_, _, c)| *c);
    for line in huffman_table {
        writeln!(&mut file, "  ({}, {}),", line.1, line.0).unwrap();
    }
    writeln!(&mut file, "];").unwrap();
}

fn make_huffman_node<W: Write>(out: &mut W, root: u32, len: u8) {
    let left = root << 1;
    let right = root << 1 | 1;
    let mut left_found = false;
    let mut right_found = false;

    writeln!(out, "{}&Node::Node(", "  ".repeat((len - 1) as usize)).unwrap();
    println!("{} {}", left, right);
    for (h, l, c) in HUFFMAN_TABLE {
        if *h == left && *l == len {
            write!(out, "{}&Node::Terminal({})", "  ".repeat(len as usize), c).unwrap();
            left_found = true;
        }
    }
    if !left_found {
        make_huffman_node(out, left, len + 1);
    }
    writeln!(out, ",").unwrap();
    for (h, l, c) in HUFFMAN_TABLE {
        if *h == right && *l == len {
            write!(out, "{}&Node::Terminal({})", "  ".repeat(len as usize), c).unwrap();
            right_found = true;
        }
    }
    if !right_found {
        make_huffman_node(out, right, len + 1);
    }
    write!(out, "\n{})", "  ".repeat((len - 1) as usize)).unwrap();
}
