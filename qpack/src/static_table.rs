#[derive(Copy, Clone, Eq, PartialEq)]
pub struct TableEntry<'a> {
    pub name: &'a [u8],
    pub value: &'a [u8],
}

impl std::fmt::Debug for TableEntry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{}={}",
            String::from_utf8_lossy(self.name),
            String::from_utf8_lossy(self.value)
        ))
    }
}

impl phf::PhfHash for TableEntry<'_> {
    #[inline]
    fn phf_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.phf_hash(state);
        self.value.phf_hash(state);
    }
}

impl<'a> phf_shared::PhfBorrow<TableEntry<'a>> for TableEntry<'static> {
    fn borrow(&self) -> &TableEntry<'a> {
        self
    }
}

include!(concat!(env!("OUT_DIR"), "/qpack_static_table.rs"));

pub fn get_entry(i: u64) -> Option<TableEntry<'static>> {
    if i >= STATIC_TABLE.len() as u64 {
        return None;
    }
    Some(STATIC_TABLE[i as usize])
}

pub fn get_index_by_name(name: &[u8]) -> Option<u64> {
    NAME_LOOKUP.get(name).map(|r| *r as u64)
}

pub fn get_index_by_name_value(name: &[u8], value: &[u8]) -> Option<u64> {
    NAME_VALUE_LOOKUP
        .get(&TableEntry { name, value })
        .map(|r| *r as u64)
}
