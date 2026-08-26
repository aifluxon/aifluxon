use crate::{
    now_millis, ProviderStateRecord, ProviderStateStore, SessionRecord, SessionStore,
    SessionSummary, StoreError,
};
use aifluxon_core::{ProviderId, SessionId};
use atomicwrites::{AllowOverwrite, AtomicFile};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone)]
pub struct JsonFileSessionStore {
    root: Arc<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct SessionEnvelope {
    schema_version: u32,
    record: SessionRecord,
}

#[derive(Serialize, Deserialize)]
struct SessionIndex {
    schema_version: u32,
    sessions: Vec<SessionSummary>,
}

impl JsonFileSessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        if root.as_os_str().is_empty() {
            return Err(StoreError::Unavailable(
                "JSON session store requires an explicit data directory.".to_string(),
            ));
        }
        Ok(Self {
            root: Arc::new(root),
        })
    }

    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    fn records_dir(&self) -> PathBuf {
        self.sessions_dir().join("records")
    }

    fn index_path(&self) -> PathBuf {
        self.sessions_dir().join("index.json")
    }

    fn lock_path(&self) -> PathBuf {
        self.sessions_dir().join("store.lock")
    }

    fn record_path(&self, id: &SessionId) -> PathBuf {
        self.records_dir().join(format!("{}.json", id.hyphenated()))
    }

    fn ensure_layout(&self) -> Result<(), StoreError> {
        fs::create_dir_all(self.records_dir()).map_err(io_error)
    }

    fn acquire_store_lock(&self) -> Result<std::fs::File, StoreError> {
        self.ensure_layout()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path())
            .map_err(io_error)?;
        file.lock_exclusive().map_err(io_error)?;
        Ok(file)
    }

    fn load_record_sync(&self, id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        self.ensure_layout()?;
        let path = self.record_path(id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        let envelope = serde_json::from_slice::<SessionEnvelope>(&bytes)
            .map_err(|_| quarantine_corrupt(&path))?;
        if envelope.schema_version != SCHEMA_VERSION || envelope.record.id != *id {
            return Err(quarantine_corrupt(&path));
        }
        Ok(Some(envelope.record))
    }

    fn list_sync(&self) -> Result<Vec<SessionSummary>, StoreError> {
        self.ensure_layout()?;
        let mut summaries = Vec::new();
        let mut entries = fs::read_dir(self.records_dir())
            .map_err(io_error)?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let Ok(uuid) = uuid::Uuid::parse_str(stem) else {
                return Err(quarantine_corrupt(&path));
            };
            let id = SessionId(uuid);
            if let Some(record) = self.load_record_sync(&id)? {
                summaries.push(SessionSummary::from(&record));
            }
        }
        summaries
            .sort_by_key(|record| (std::cmp::Reverse(record.updated_at), record.id.hyphenated()));
        Ok(summaries)
    }

    fn write_index_sync(&self) -> Result<(), StoreError> {
        let index = SessionIndex {
            schema_version: SCHEMA_VERSION,
            sessions: self.list_sync()?,
        };
        atomic_write_json(&self.index_path(), &index)
    }
}

#[async_trait::async_trait]
impl SessionStore for JsonFileSessionStore {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        let store = self.clone();
        let id = *id;
        tokio::task::spawn_blocking(move || {
            let _store_lock = store.acquire_store_lock()?;
            store.load_record_sync(&id)
        })
        .await
        .map_err(task_error)?
    }

    async fn save(&self, mut record: SessionRecord) -> Result<SessionRecord, StoreError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _store_lock = store.acquire_store_lock()?;
            store.ensure_layout()?;
            match store.load_record_sync(&record.id)? {
                Some(existing) if existing.revision != record.revision => {
                    return Err(StoreError::Conflict)
                }
                None if record.revision != 0 => return Err(StoreError::Conflict),
                _ => {}
            }
            record.revision = record.revision.saturating_add(1);
            record.updated_at = now_millis();
            atomic_write_json(
                &store.record_path(&record.id),
                &SessionEnvelope {
                    schema_version: SCHEMA_VERSION,
                    record: record.clone(),
                },
            )?;
            store.write_index_sync()?;
            Ok(record)
        })
        .await
        .map_err(task_error)?
    }

    async fn delete(&self, id: &SessionId) -> Result<(), StoreError> {
        let store = self.clone();
        let id = *id;
        tokio::task::spawn_blocking(move || {
            let _store_lock = store.acquire_store_lock()?;
            store.ensure_layout()?;
            let path = store.record_path(&id);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
            store.write_index_sync()
        })
        .await
        .map_err(task_error)?
    }

    async fn list(&self) -> Result<Vec<SessionSummary>, StoreError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _store_lock = store.acquire_store_lock()?;
            let summaries = store.list_sync()?;
            atomic_write_json(
                &store.index_path(),
                &SessionIndex {
                    schema_version: SCHEMA_VERSION,
                    sessions: summaries.clone(),
                },
            )?;
            Ok(summaries)
        })
        .await
        .map_err(task_error)?
    }
}

#[derive(Clone)]
pub struct JsonFileProviderStateStore {
    root: Arc<PathBuf>,
}

impl JsonFileProviderStateStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        if root.as_os_str().is_empty() {
            return Err(StoreError::Unavailable(
                "JSON provider-state store requires an explicit data directory.".to_string(),
            ));
        }
        Ok(Self {
            root: Arc::new(root),
        })
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join("provider-state")
    }

    fn lock_path(&self) -> PathBuf {
        self.state_dir().join("store.lock")
    }

    fn record_path(&self, session_id: &SessionId, provider_id: &ProviderId) -> PathBuf {
        self.state_dir().join(format!(
            "{}--{}.json",
            session_id.hyphenated(),
            sanitize_provider_stem(provider_id)
        ))
    }

    fn acquire_store_lock(&self) -> Result<std::fs::File, StoreError> {
        fs::create_dir_all(self.state_dir()).map_err(io_error)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.lock_path())
            .map_err(io_error)?;
        file.lock_exclusive().map_err(io_error)?;
        Ok(file)
    }

    fn load_sync(
        &self,
        session_id: &SessionId,
        provider_id: &ProviderId,
    ) -> Result<Option<ProviderStateRecord>, StoreError> {
        fs::create_dir_all(self.state_dir()).map_err(io_error)?;
        let path = self.record_path(session_id, provider_id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        let record = serde_json::from_slice::<ProviderStateRecord>(&bytes)
            .map_err(|_| quarantine_corrupt(&path))?;
        if record.session_id != *session_id || record.provider_id != *provider_id {
            return Err(quarantine_corrupt(&path));
        }
        Ok(Some(record))
    }
}

#[async_trait::async_trait]
impl ProviderStateStore for JsonFileProviderStateStore {
    async fn load(
        &self,
        session_id: &SessionId,
        provider_id: &ProviderId,
    ) -> Result<Option<ProviderStateRecord>, StoreError> {
        let store = self.clone();
        let session_id = *session_id;
        let provider_id = provider_id.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = store.acquire_store_lock()?;
            store.load_sync(&session_id, &provider_id)
        })
        .await
        .map_err(task_error)?
    }

    async fn save(&self, record: ProviderStateRecord) -> Result<(), StoreError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = store.acquire_store_lock()?;
            atomic_write_json(
                &store.record_path(&record.session_id, &record.provider_id),
                &record,
            )
        })
        .await
        .map_err(task_error)?
    }

    async fn delete(
        &self,
        session_id: &SessionId,
        provider_id: &ProviderId,
    ) -> Result<(), StoreError> {
        let store = self.clone();
        let session_id = *session_id;
        let provider_id = provider_id.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = store.acquire_store_lock()?;
            let path = store.record_path(&session_id, &provider_id);
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(io_error(error)),
            }
        })
        .await
        .map_err(task_error)?
    }
}

fn sanitize_provider_stem(provider_id: &ProviderId) -> String {
    let stem = provider_id
        .as_str()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if stem.is_empty() {
        "provider".to_string()
    } else {
        stem
    }
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::Unavailable("state path has no parent".to_string()))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let payload = serde_json::to_vec_pretty(value)
        .map_err(|error| StoreError::Unavailable(error.to_string()))?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(&payload)?;
            file.write_all(b"\n")?;
            file.flush()?;
            file.sync_all()
        })
        .map_err(|error| StoreError::Unavailable(error.to_string()))
}

fn quarantine_corrupt(path: &Path) -> StoreError {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("record");
    let quarantine = path.with_file_name(format!("{name}.corrupt-{}", now_millis()));
    match fs::rename(path, &quarantine) {
        Ok(()) => StoreError::CorruptQuarantined(quarantine.display().to_string()),
        Err(error) => {
            StoreError::Unavailable(format!("corrupt record could not be quarantined: {error}"))
        }
    }
}

fn io_error(error: std::io::Error) -> StoreError {
    StoreError::Unavailable(error.to_string())
}

fn task_error(error: tokio::task::JoinError) -> StoreError {
    StoreError::Unavailable(format!("state store worker failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "aifluxon-json-store-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn json_store_round_trips_across_instances_and_enforces_cas() {
        let root = temp_root("roundtrip");
        let first = JsonFileSessionStore::new(&root).unwrap();
        let saved = first
            .save(SessionRecord::new(SessionId::new()))
            .await
            .unwrap();
        let reopened = JsonFileSessionStore::new(&root).unwrap();
        assert_eq!(reopened.load(&saved.id).await.unwrap().unwrap(), saved);
        let mut stale = saved.clone();
        let mut current = saved.clone();
        current.messages.push(aifluxon_core::Message {
            role: aifluxon_core::MessageRole::User,
            content: vec![aifluxon_core::ContentPart::Text("persisted".to_string())],
            tool_calls: Vec::new(),
            tool_call_id: None,
            provider_state: None,
        });
        let current = reopened.save(current).await.unwrap();
        stale.messages.clear();
        assert_eq!(
            reopened.save(stale).await.unwrap_err(),
            StoreError::Conflict
        );
        assert_eq!(current.revision, 2);
        assert!(root.join("sessions/index.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn corrupt_record_is_quarantined_and_never_returned() {
        let root = temp_root("corrupt");
        let store = JsonFileSessionStore::new(&root).unwrap();
        let id = SessionId::new();
        store.ensure_layout().unwrap();
        fs::write(store.record_path(&id), b"{broken").unwrap();
        assert!(matches!(
            store.load(&id).await,
            Err(StoreError::CorruptQuarantined(_))
        ));
        assert!(!store.record_path(&id).exists());
        assert!(fs::read_dir(store.records_dir())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-")));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn independent_instances_serialize_mutations_and_reject_stale_writes() {
        let root = temp_root("concurrent-cas");
        let first = JsonFileSessionStore::new(&root).unwrap();
        let saved = first
            .save(SessionRecord::new(SessionId::new()))
            .await
            .unwrap();
        let left = JsonFileSessionStore::new(&root).unwrap();
        let right = JsonFileSessionStore::new(&root).unwrap();
        let mut left_record = saved.clone();
        let mut right_record = saved.clone();
        left_record.metadata.insert(
            "writer".to_string(),
            serde_json::Value::String("left".to_string()),
        );
        right_record.metadata.insert(
            "writer".to_string(),
            serde_json::Value::String("right".to_string()),
        );

        let (left_result, right_result) =
            tokio::join!(left.save(left_record), right.save(right_record));
        assert_ne!(left_result.is_ok(), right_result.is_ok());
        assert!(matches!(
            left_result.as_ref().err().or(right_result.as_ref().err()),
            Some(StoreError::Conflict)
        ));

        let reopened = JsonFileSessionStore::new(&root).unwrap();
        assert_eq!(reopened.load(&saved.id).await.unwrap().unwrap().revision, 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn provider_state_is_isolated_by_provider_id_and_survives_reopen() {
        let root = temp_root("provider-state");
        let store = JsonFileProviderStateStore::new(&root).unwrap();
        let session = SessionId::new();
        store
            .save(ProviderStateRecord {
                session_id: session,
                provider_id: ProviderId::new("openai"),
                state: serde_json::json!({ "cursor": 1 }),
            })
            .await
            .unwrap();
        store
            .save(ProviderStateRecord {
                session_id: session,
                provider_id: ProviderId::new("deepseek"),
                state: serde_json::json!({ "cursor": 9 }),
            })
            .await
            .unwrap();
        let reopened = JsonFileProviderStateStore::new(&root).unwrap();
        assert_eq!(
            reopened
                .load(&session, &ProviderId::new("openai"))
                .await
                .unwrap()
                .unwrap()
                .state,
            serde_json::json!({ "cursor": 1 })
        );
        assert_eq!(
            reopened
                .load(&session, &ProviderId::new("deepseek"))
                .await
                .unwrap()
                .unwrap()
                .state,
            serde_json::json!({ "cursor": 9 })
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_record_json_does_not_define_credential_fields() {
        let encoded = serde_json::to_value(SessionRecord::new(SessionId::new())).unwrap();
        let text = encoded.to_string().to_ascii_lowercase();
        for forbidden in ["api_key", "apikey", "oauth", "cookie", "secret", "token"] {
            assert!(
                !text.contains(forbidden),
                "session JSON must not contain `{forbidden}`"
            );
        }
    }
}
