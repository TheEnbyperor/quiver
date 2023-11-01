use super::Header;

#[derive(Debug)]
struct Entry {
    header: Header<'static>,
    references: u64,
}

#[derive(Debug)]
pub struct DynamicTable {
    storage: std::collections::VecDeque<Entry>,
    capacity: u64,
    maximum_capacity: u64,
    acknowledged_index: u64,
    evicted_index: u64,
}

impl DynamicTable {
    pub fn new() -> Self {
        Self {
            storage: std::collections::VecDeque::new(),
            capacity: 0,
            maximum_capacity: 0,
            acknowledged_index: 0,
            evicted_index: 0,
        }
    }
    fn size(&self) -> u64 {
        self.storage.iter().map(|e| e.header.size()).sum()
    }

    pub fn insert_count(&self) -> u64 {
        self.storage.len() as u64 + self.evicted_index
    }

    pub fn max_entries(&self) -> u64 {
        self.maximum_capacity / 32
    }

    pub fn get(&self, mut index: u64) -> Option<Header<'static>> {
        if index < self.evicted_index {
            return None;
        }
        index += self.evicted_index;
        if index > self.storage.len() as u64 {
            return None;
        }
        Some(self.storage[index as usize].header.make_static())
    }

    pub fn get_index_by_name(&self, name: &[u8]) -> Option<u64> {
        for (i, h) in self.storage.iter().enumerate().rev() {
            if h.header.name == name {
                return Some(i as u64 + self.evicted_index);
            }
        }
        None
    }

    pub fn get_index_by_name_value(&self, name: &[u8], value: &[u8]) -> Option<u64> {
        for (i, h) in self.storage.iter().enumerate().rev() {
            if h.header.name == name && h.header.value == value {
                return Some(i as u64 + self.evicted_index);
            }
        }
        None
    }

    fn evict_count(&self, header: &Header, force_evict: bool) -> Option<usize> {
        let header_size = header.size();
        let mut size = self.size();
        let mut i = 0usize;
        while (header_size + size) > self.capacity {
            if !force_evict && i == self.storage.len() {
                return None;
            }
            let abs_index = i as u64 + self.evicted_index;
            if !force_evict && abs_index > self.acknowledged_index {
                return None;
            }
            let entry = self.storage.get(i).unwrap();
            if !force_evict && entry.references > 0 {
                return None;
            }
            size -= entry.header.size();
            i += 1;
        }
        Some(i)
    }

    pub fn can_index(&self, header: &Header) -> bool {
        self.evict_count(header, false).is_some()
    }

    pub fn set_capacity(&mut self, new_capacity: u64) {
        let mut size = self.size();
        let mut i = 0usize;
        while size > new_capacity {
            let entry = self.storage.get(i).unwrap();
            size -= entry.header.size();
            i += 1;
        }
        self.storage.drain(..i);
        self.evicted_index += i as u64;
    }

    pub fn add(&mut self, header: Header<'static>, force_evict: bool) -> u64 {
        let evict_count = self
            .evict_count(&header, force_evict)
            .expect("No space left");
        self.storage.drain(..evict_count);
        self.evicted_index += evict_count as u64;
        let i = self.storage.len() as u64;
        self.storage.push_front(Entry {
            header,
            references: 0,
        });
        i + self.evicted_index
    }

    pub fn acknowledged(&mut self, index: u64) {
        self.acknowledged_index = std::cmp::max(self.acknowledged_index, index);
    }

    pub fn increment_acknowledged(&mut self, inc: u64) {
        self.acknowledged_index += inc;
    }

    pub fn increment_references(&mut self, index: u64) {
        if index < self.evicted_index {
            return;
        }
        let i = (index - self.evicted_index) as usize;
        if i > self.storage.len() {
            return;
        }
        self.storage.get_mut(i).unwrap().references += 1;
    }

    pub fn decrement_references(&mut self, index: u64) {
        if index < self.evicted_index {
            return;
        }
        let i = (index - self.evicted_index) as usize;
        if i > self.storage.len() {
            return;
        }
        self.storage.get_mut(i).unwrap().references -= 1;
    }
}
