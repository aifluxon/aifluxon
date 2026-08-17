use aifluxon_core::{Message, ProviderId, RunId, RunState, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub type TimestampMillis = u64;

pub fn now_millis() -> TimestampMillis {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub revision: u64,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

impl SessionRecord {
    pub fn new(id: SessionId) -> Self {
        let now = now_millis();
        Self {
            id,
            revision: 0,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub revision: u64,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub message_count: usize,
}

impl From<&SessionRecord> for SessionSummary {
    fn from(record: &SessionRecord) -> Self {
        Self {
            id: record.id,
            revision: record.revision,
            created_at: record.created_at,
            updated_at: record.updated_at,
            message_count: record.messages.len(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProviderStateRecord {
    pub session_id: SessionId,
    pub provider_id: ProviderId,
    pub state: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RunCheckpoint {
    pub run_id: RunId,
    pub session_id: Option<SessionId>,
    pub state: RunState,
    pub messages: Vec<Message>,
    pub provider_state: Option<Value>,
    pub updated_at: TimestampMillis,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StoreError {
    #[error("The state store is unavailable: {0}")]
    Unavailable(String),
    #[error("The session revision is stale.")]
    Conflict,
    #[error("The stored record was corrupt and quarantined: {0}")]
    CorruptQuarantined(String),
    #[error("The state identifier is invalid.")]
    InvalidId,
}

#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionRecord>, StoreError>;
    async fn save(&self, record: SessionRecord) -> Result<SessionRecord, StoreError>;
    async fn delete(&self, id: &SessionId) -> Result<(), StoreError>;
    async fn list(&self) -> Result<Vec<SessionSummary>, StoreError>;
}

#[async_trait::async_trait]
pub trait ProviderStateStore: Send + Sync {
    async fn load(
        &self,
        session_id: &SessionId,
        provider_id: &ProviderId,
    ) -> Result<Option<ProviderStateRecord>, StoreError>;
    async fn save(&self, record: ProviderStateRecord) -> Result<(), StoreError>;
    async fn delete(
        &self,
        session_id: &SessionId,
        provider_id: &ProviderId,
    ) -> Result<(), StoreError>;
}

#[async_trait::async_trait]
pub trait RunCheckpointStore: Send + Sync {
    async fn load(&self, run_id: &RunId) -> Result<Option<RunCheckpoint>, StoreError>;
    async fn save(&self, checkpoint: RunCheckpoint) -> Result<(), StoreError>;
    async fn delete(&self, run_id: &RunId) -> Result<(), StoreError>;
}

#[derive(Clone, Default)]
pub struct InMemorySessionStore {
    records: Arc<Mutex<HashMap<SessionId, SessionRecord>>>,
}

#[async_trait::async_trait]
impl SessionStore for InMemorySessionStore {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        Ok(self.records.lock().map_err(lock_error)?.get(id).cloned())
    }

    async fn save(&self, mut record: SessionRecord) -> Result<SessionRecord, StoreError> {
        let mut records = self.records.lock().map_err(lock_error)?;
        match records.get(&record.id) {
            Some(existing) if existing.revision != record.revision => {
                return Err(StoreError::Conflict)
            }
            None if record.revision != 0 => return Err(StoreError::Conflict),
            _ => {}
        }
        record.revision = record.revision.saturating_add(1);
        record.updated_at = now_millis();
        records.insert(record.id, record.clone());
        Ok(record)
    }

    async fn delete(&self, id: &SessionId) -> Result<(), StoreError> {
        self.records.lock().map_err(lock_error)?.remove(id);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<SessionSummary>, StoreError> {
        let mut values = self
            .records
            .lock()
            .map_err(lock_error)?
            .values()
            .map(SessionSummary::from)
            .collect::<Vec<_>>();
        values.sort_by_key(|record| (std::cmp::Reverse(record.updated_at), record.id.hyphenated()));
        Ok(values)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryProviderStateStore {
    records: Arc<Mutex<HashMap<(SessionId, ProviderId), ProviderStateRecord>>>,
}

#[async_trait::async_trait]
impl ProviderStateStore for InMemoryProviderStateStore {
    async fn load(
        &self,
        session_id: &SessionId,
        provider_id: &ProviderId,
    ) -> Result<Option<ProviderStateRecord>, StoreError> {
        Ok(self
            .records
            .lock()
            .map_err(lock_error)?
            .get(&(*session_id, provider_id.clone()))
            .cloned())
    }

    async fn save(&self, record: ProviderStateRecord) -> Result<(), StoreError> {
        self.records
            .lock()
            .map_err(lock_error)?
            .insert((record.session_id, record.provider_id.clone()), record);
        Ok(())
    }

    async fn delete(
        &self,
        session_id: &SessionId,
        provider_id: &ProviderId,
    ) -> Result<(), StoreError> {
        self.records
            .lock()
            .map_err(lock_error)?
            .remove(&(*session_id, provider_id.clone()));
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct InMemoryRunCheckpointStore {
    records: Arc<Mutex<HashMap<RunId, RunCheckpoint>>>,
}

#[async_trait::async_trait]
impl RunCheckpointStore for InMemoryRunCheckpointStore {
    async fn load(&self, run_id: &RunId) -> Result<Option<RunCheckpoint>, StoreError> {
        Ok(self
            .records
            .lock()
            .map_err(lock_error)?
            .get(run_id)
            .cloned())
    }

    async fn save(&self, checkpoint: RunCheckpoint) -> Result<(), StoreError> {
        self.records
            .lock()
            .map_err(lock_error)?
            .insert(checkpoint.run_id, checkpoint);
        Ok(())
    }

    async fn delete(&self, run_id: &RunId) -> Result<(), StoreError> {
        self.records.lock().map_err(lock_error)?.remove(run_id);
        Ok(())
    }
}

fn lock_error<T>(_error: std::sync::PoisonError<T>) -> StoreError {
    StoreError::Unavailable("state lock was poisoned".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_session_store_enforces_revision_cas() {
        let store = InMemorySessionStore::default();
        let original = SessionRecord::new(SessionId::new());
        let saved = store.save(original.clone()).await.unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(
            store.save(original).await.unwrap_err(),
            StoreError::Conflict
        );
        assert_eq!(store.load(&saved.id).await.unwrap().unwrap(), saved);
    }

    #[tokio::test]
    async fn provider_state_is_opaque_and_isolated_by_session_and_provider() {
        let store = InMemoryProviderStateStore::default();
        let session = SessionId::new();
        let other = SessionId::new();
        let provider = ProviderId::new("private-web");
        store
            .save(ProviderStateRecord {
                session_id: session,
                provider_id: provider.clone(),
                state: serde_json::json!({"conversation_id":"opaque"}),
            })
            .await
            .unwrap();
        assert!(store.load(&session, &provider).await.unwrap().is_some());
        assert!(store.load(&other, &provider).await.unwrap().is_none());
        assert!(store
            .load(&session, &ProviderId::new("other"))
            .await
            .unwrap()
            .is_none());
    }
}
