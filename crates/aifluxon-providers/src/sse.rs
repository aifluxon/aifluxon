//! Backward-compatible module path; the implementation lives in `common`.

pub use crate::common::sse::{IncrementalSseParser, SseEvent};
pub use crate::common::utf8::Utf8ChunkDecoder;
