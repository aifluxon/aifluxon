pub use aifluxon_core::{RunEvent, RunEventEnvelope};

pub trait RunEventSink: Send + Sync {
    fn emit(&self, event: RunEventEnvelope);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopRunEventSink;

impl RunEventSink for NoopRunEventSink {
    fn emit(&self, _event: RunEventEnvelope) {}
}

/// Transport-neutral event stream. Channel implementation details remain private.
pub struct RunEventStream {
    receiver: tokio::sync::mpsc::Receiver<RunEventEnvelope>,
}

impl RunEventStream {
    pub(crate) fn from_receiver(receiver: tokio::sync::mpsc::Receiver<RunEventEnvelope>) -> Self {
        Self { receiver }
    }
    pub async fn next(&mut self) -> Option<RunEventEnvelope> {
        self.receiver.recv().await
    }

    #[cfg(test)]
    pub(crate) fn closed() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(sender);
        Self { receiver }
    }
}

impl std::fmt::Debug for RunEventStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunEventStream")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_stream_ends_without_exposing_channel_types() {
        let mut stream = RunEventStream::closed();
        assert_eq!(stream.next().await, None);
    }
}
