#[derive(Default)]
pub struct Utf8ChunkDecoder {
    pending: Vec<u8>,
}

impl Utf8ChunkDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let mut decoded = String::new();
        loop {
            let (valid_up_to, error_len) = match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    decoded.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(error) => (error.valid_up_to(), error.error_len()),
            };
            if valid_up_to > 0 {
                decoded.push_str(
                    std::str::from_utf8(&self.pending[..valid_up_to])
                        .expect("validated UTF-8 prefix"),
                );
                self.pending.drain(..valid_up_to);
            }
            let Some(error_len) = error_len else {
                break;
            };
            decoded.push('\u{fffd}');
            self.pending.drain(..error_len);
        }
        decoded
    }

    pub fn finish(&mut self) -> String {
        let mut decoded = self.push(&[]);
        if !self.pending.is_empty() {
            decoded.push_str(&String::from_utf8_lossy(&self.pending));
            self.pending.clear();
        }
        decoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_utf8_chunk_split_round_trips() {
        let source = "模型🙂stream";
        for split in 0..=source.len() {
            if split > source.len() {
                continue;
            }
            let mut decoder = Utf8ChunkDecoder::default();
            let mut output = decoder.push(&source.as_bytes()[..split]);
            output.push_str(&decoder.push(&source.as_bytes()[split..]));
            output.push_str(&decoder.finish());
            assert_eq!(output, source);
            assert!(!output.contains('\u{fffd}'));
        }
    }
}
