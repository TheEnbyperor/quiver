use super::{dynamic_table, error, field_lines, instructions, static_table, Header, Headers};

#[derive(Debug)]
pub struct Decoder {
    dynamic_table: dynamic_table::DynamicTable,
    pending_commands: Vec<instructions::DecoderInstruction>,
    pending_streams: Vec<(u64, std::sync::Arc<tokio::sync::Notify>)>,
}

pub enum DecodeResult {
    Wait(std::sync::Arc<tokio::sync::Notify>),
    Headers(Headers<'static>),
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            dynamic_table: dynamic_table::DynamicTable::new(),
            pending_commands: Vec::new(),
            pending_streams: Vec::new(),
        }
    }

    pub fn has_pending_decoder_commands(&self) -> bool {
        !self.pending_commands.is_empty()
    }

    pub fn pending_decoder_commands(&mut self) -> Vec<instructions::DecoderInstruction> {
        self.pending_commands.drain(..).collect()
    }

    pub fn handle_encoder_instruction(
        &mut self,
        instruction: instructions::EncoderInstruction,
    ) -> error::QPackResult<()> {
        match instruction {
            instructions::EncoderInstruction::SetDynamicTableCapacity(capacity) => {
                self.dynamic_table.set_capacity(capacity)
            }
            instructions::EncoderInstruction::InsertNameReference { name_index, value } => {
                dbg!(name_index, value);
            }
            instructions::EncoderInstruction::InsertLiteralName { name, value } => {
                dbg!(name, value);
                // self.dynamic_table.add(Header::from_bytes(), true);
            }
            instructions::EncoderInstruction::Duplicate(index) => {
                let val = self
                    .dynamic_table
                    .get(index)
                    .ok_or(error::QPackError::DecoderStreamError)?;
                self.dynamic_table.add(val, true);
            }
        }

        let insert_count = self.dynamic_table.insert_count();
        let readable = self
            .pending_streams
            .extract_if(|(riq, _)| *riq >= insert_count)
            .collect::<Vec<_>>();
        for (_, notify) in readable {
            notify.notify_waiters();
        }

        Ok(())
    }

    fn get_field(
        &self,
        index: field_lines::FieldIndex,
        base: u64,
        post_base: bool,
    ) -> error::QPackResult<Header<'static>> {
        match index {
            field_lines::FieldIndex::Static(i) => match static_table::get_entry(i) {
                Some(e) => Ok(Header::from_static(e.name, e.value)),
                None => Err(error::QPackError::DecompressionFailed),
            },
            field_lines::FieldIndex::Dynamic(i) => {
                match self
                    .dynamic_table
                    .get(if post_base { base - i } else { base + i })
                {
                    Some(e) => Ok(e),
                    None => Err(error::QPackError::DecompressionFailed),
                }
            }
        }
    }

    pub fn decode_field(
        &mut self,
        stream_id: u64,
        field_lines: field_lines::FieldLines,
    ) -> error::QPackResult<DecodeResult> {
        let max_entries = self.dynamic_table.max_entries();
        let full_range = 2 * max_entries;
        let required_insert_count = if field_lines.required_insert_count == 0 {
            0
        } else {
            if field_lines.required_insert_count > full_range {
                return Err(error::QPackError::DecompressionFailed);
            }
            let max_value = self.dynamic_table.insert_count() + max_entries;
            let max_wrapped = (max_value / full_range) * full_range;
            let mut required_insert_count = max_wrapped + field_lines.required_insert_count - 1;

            if required_insert_count > max_value {
                if required_insert_count <= full_range {
                    return Err(error::QPackError::DecompressionFailed);
                }
                required_insert_count -= full_range;
            }

            if required_insert_count == 0 {
                return Err(error::QPackError::DecompressionFailed);
            }

            required_insert_count
        };

        if required_insert_count < self.dynamic_table.insert_count() {
            let notify = std::sync::Arc::new(tokio::sync::Notify::new());
            self.pending_streams
                .push((required_insert_count, notify.clone()));
            return Ok(DecodeResult::Wait(notify));
        }

        let base = (required_insert_count as i64 + field_lines.delta_base) as u64;

        let mut headers = Headers::new();

        for field_line in field_lines.lines {
            match field_line {
                field_lines::FieldLine::Indexed(i) => {
                    headers.headers.push(self.get_field(i, base, false)?);
                }
                field_lines::FieldLine::PostBaseIndexed(i) => {
                    headers.headers.push(self.get_field(
                        field_lines::FieldIndex::Dynamic(i),
                        base,
                        true,
                    )?);
                }
                field_lines::FieldLine::LiteralWithNameReference {
                    must_be_literal: _,
                    name_index,
                    value,
                } => {
                    let mut field = self.get_field(name_index, base, false)?;
                    field.value = std::borrow::Cow::Owned(
                        value
                            .to_vec()
                            .map_err(|_| error::QPackError::DecompressionFailed)?,
                    );
                    headers.headers.push(field);
                }
                field_lines::FieldLine::LiteralWithPostBaseNameReference {
                    must_be_literal: _,
                    name_index,
                    value,
                } => {
                    let mut field =
                        self.get_field(field_lines::FieldIndex::Dynamic(name_index), base, true)?;
                    field.value = std::borrow::Cow::Owned(
                        value
                            .to_vec()
                            .map_err(|_| error::QPackError::DecompressionFailed)?,
                    );
                    headers.headers.push(field);
                }
                field_lines::FieldLine::Literal {
                    must_be_literal: _,
                    name,
                    value,
                } => {
                    headers.headers.push(Header {
                        name: std::borrow::Cow::Owned(
                            name.to_vec()
                                .map_err(|_| error::QPackError::DecompressionFailed)?,
                        ),
                        value: std::borrow::Cow::Owned(
                            value
                                .to_vec()
                                .map_err(|_| error::QPackError::DecompressionFailed)?,
                        ),
                    });
                }
            }
        }

        self.pending_commands
            .push(instructions::DecoderInstruction::SectionAcknowledgment(
                stream_id,
            ));

        Ok(DecodeResult::Headers(headers))
    }
}
