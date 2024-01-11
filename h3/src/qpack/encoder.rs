use super::{
    dynamic_table, error, field_lines, huffman, instructions, static_table, Header, Headers,
};

#[derive(Debug)]
pub struct Encoder {
    dynamic_table: dynamic_table::DynamicTable,
    pending_commands: Vec<instructions::EncoderInstruction>,
    stream_index: std::collections::HashMap<u64, std::collections::VecDeque<(u64, Vec<u64>)>>,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Self {
            dynamic_table: dynamic_table::DynamicTable::new(),
            pending_commands: Vec::new(),
            stream_index: std::collections::HashMap::new(),
        }
    }

    fn should_index(_line: &Header) -> bool {
        // TODO: actually decide whats worth indexing
        true
    }

    pub fn set_dynamic_table_capacity(&mut self, capacity: u64) {
        self.dynamic_table.set_capacity(capacity);
        let inst = instructions::EncoderInstruction::SetDynamicTableCapacity(capacity);
        self.pending_commands.push(inst)
    }

    pub fn has_pending_encoder_commands(&self) -> bool {
        !self.pending_commands.is_empty()
    }

    /// Drains pending control commands from the encoder's state
    pub fn pending_encoder_commands(&mut self) -> Vec<instructions::EncoderInstruction> {
        self.pending_commands.drain(..).collect()
    }

    pub fn handle_decoder_instruction(
        &mut self,
        instruction: instructions::DecoderInstruction,
    ) -> error::QPackResult<()> {
        match instruction {
            instructions::DecoderInstruction::SectionAcknowledgment(stream_id) => {
                let s = self
                    .stream_index
                    .get_mut(&stream_id)
                    .ok_or(error::QPackError::DecoderStreamError)?;
                let (riq, refs) = s.pop_back().ok_or(error::QPackError::DecoderStreamError)?;
                self.dynamic_table.acknowledged(riq);
                for i in refs {
                    self.dynamic_table.decrement_references(i);
                }
            }
            instructions::DecoderInstruction::StreamCancellation(stream_id) => {
                let s = self
                    .stream_index
                    .get_mut(&stream_id)
                    .ok_or(error::QPackError::DecoderStreamError)?;
                for (_, refs) in s.drain(..) {
                    for i in refs {
                        self.dynamic_table.decrement_references(i);
                    }
                }
            }
            instructions::DecoderInstruction::InsertCountIncrement(increment) => {
                self.dynamic_table.increment_acknowledged(increment);
            }
        }
        Ok(())
    }

    fn encode_string(data: &[u8]) -> field_lines::FieldString {
        let huffman_encoded = huffman::encode(data);
        if huffman_encoded.len() < data.len() {
            field_lines::FieldString::Huffman(huffman_encoded)
        } else {
            field_lines::FieldString::Raw(data.to_vec())
        }
    }

    pub fn encode_field(
        &mut self,
        stream_id: u64,
        headers: &Headers<'_>,
    ) -> field_lines::FieldLines {
        let base = self.dynamic_table.insert_count();
        let mut required_insert_count = 0;
        let mut lines = vec![];
        let mut references = vec![];

        for line in &headers.headers {
            if let Some(i) = static_table::get_index_by_name_value(&line.name, &line.value) {
                lines.push(field_lines::FieldLine::Indexed(
                    field_lines::FieldIndex::Static(i),
                ));
                continue;
            }

            if let Some(i) = self
                .dynamic_table
                .get_index_by_name_value(&line.name, &line.value)
            {
                required_insert_count = std::cmp::max(required_insert_count, i);
                self.dynamic_table.increment_references(i);
                references.push(i);
                lines.push(field_lines::FieldLine::Indexed(
                    field_lines::FieldIndex::Dynamic(i - base),
                ));
                continue;
            }

            let name_index = if let Some(i) = static_table::get_index_by_name(&line.name) {
                Some(field_lines::FieldIndex::Static(i))
            } else if let Some(i) = self.dynamic_table.get_index_by_name(&line.name) {
                required_insert_count = std::cmp::max(required_insert_count, i);
                self.dynamic_table.increment_references(i);
                references.push(i);
                Some(field_lines::FieldIndex::Dynamic(i - base))
            } else {
                None
            };

            if Self::should_index(line) && self.dynamic_table.can_index(line) {
                if let Some(i) = name_index {
                    self.pending_commands.push(
                        instructions::EncoderInstruction::InsertNameReference {
                            name_index: i,
                            value: Self::encode_string(&line.value),
                        },
                    );
                } else {
                    self.pending_commands.push(
                        instructions::EncoderInstruction::InsertLiteralName {
                            name: Self::encode_string(&line.name),
                            value: Self::encode_string(&line.value),
                        },
                    );
                }
                let dynamic_index = self.dynamic_table.add(line.make_static(), false);

                required_insert_count = std::cmp::max(required_insert_count, dynamic_index);
                self.dynamic_table.increment_references(dynamic_index);
                references.push(dynamic_index);
                lines.push(field_lines::FieldLine::Indexed(
                    field_lines::FieldIndex::Dynamic(dynamic_index - base),
                ));
            } else if let Some(i) = name_index {
                lines.push(field_lines::FieldLine::LiteralWithNameReference {
                    must_be_literal: false,
                    name_index: i,
                    value: Self::encode_string(&line.value),
                });
            } else {
                lines.push(field_lines::FieldLine::Literal {
                    must_be_literal: false,
                    name: Self::encode_string(&line.name),
                    value: Self::encode_string(&line.value),
                });
            }
        }

        let (wire_ric, delta_base) = if required_insert_count == 0 {
            (0, 0)
        } else {
            let wire_ric = (required_insert_count % (2 * self.dynamic_table.max_entries())) + 1;
            let delta_base = base as i64 - required_insert_count as i64;
            (wire_ric, delta_base)
        };

        if required_insert_count != 0 {
            self.stream_index.entry(stream_id).or_default();
            self.stream_index
                .get_mut(&stream_id)
                .unwrap()
                .push_front((required_insert_count, references));
        }

        field_lines::FieldLines {
            required_insert_count: wire_ric,
            delta_base,
            lines,
        }
    }
}
