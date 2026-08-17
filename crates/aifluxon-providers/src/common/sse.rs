use super::Utf8ChunkDecoder;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<String>,
}

impl SseEvent {
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]"
    }
}

#[derive(Default)]
pub struct IncrementalSseParser {
    utf8: Utf8ChunkDecoder,
    buffer: String,
}

impl IncrementalSseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.push_str(&self.utf8.push(chunk));
        self.drain_complete_events()
    }

    pub fn finish(&mut self) -> Vec<SseEvent> {
        self.buffer.push_str(&self.utf8.finish());
        let mut events = self.drain_complete_events();
        if let Some(event) = parse_event_block(&std::mem::take(&mut self.buffer)) {
            events.push(event);
        }
        events
    }

    fn drain_complete_events(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        while let Some(end) = next_event_boundary(&self.buffer) {
            let block = self.buffer[..end].to_string();
            self.buffer = self.buffer[end..].to_string();
            if let Some(event) = parse_event_block(&block) {
                events.push(event);
            }
        }
        events
    }
}

fn next_event_boundary(buffer: &str) -> Option<usize> {
    match (
        buffer.find("\n\n").map(|index| index + 2),
        buffer.find("\r\n\r\n").map(|index| index + 4),
    ) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn parse_event_block(block: &str) -> Option<SseEvent> {
    let mut event = SseEvent::default();
    let mut saw_field = false;
    for raw_line in block.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = line
            .split_once(':')
            .map(|(field, value)| (field, value.strip_prefix(' ').unwrap_or(value)))
            .unwrap_or((line, ""));
        saw_field = true;
        match field {
            "data" => {
                if !event.data.is_empty() {
                    event.data.push('\n');
                }
                event.data.push_str(value);
            }
            "event" => event.event = Some(value.to_string()),
            "id" => event.id = Some(value.to_string()),
            "retry" => event.retry = Some(value.to_string()),
            _ => {}
        }
    }
    saw_field.then_some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all_splits(source: &str) -> Vec<Vec<SseEvent>> {
        (0..=source.len())
            .map(|split| {
                let mut parser = IncrementalSseParser::default();
                let mut events = parser.push(&source.as_bytes()[..split]);
                events.extend(parser.push(&source.as_bytes()[split..]));
                events.extend(parser.finish());
                events
            })
            .collect()
    }

    #[test]
    fn split_crlf_multiline_and_eof_fixtures_are_stable() {
        let source = "event: delta\r\nid: 7\r\ndata: 模型\r\ndata: output\r\n\r\n";
        let expected = vec![SseEvent {
            event: Some("delta".to_string()),
            data: "模型\noutput".to_string(),
            id: Some("7".to_string()),
            retry: None,
        }];
        for events in parse_all_splits(source) {
            assert_eq!(events, expected);
        }

        let mut eof = IncrementalSseParser::default();
        assert!(eof.push(b"data: trailing").is_empty());
        assert_eq!(eof.finish()[0].data, "trailing");
    }
}
