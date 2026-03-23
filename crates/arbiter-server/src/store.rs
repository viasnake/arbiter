use arbiter_config::Config;
use arbiter_contracts::{ContractsMetadata, RunEnvelope};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::audit::{append_audit_record, AuditRecord};
use crate::contracts::build_contracts_metadata;
use crate::errors::ApiFailure;

#[derive(Clone)]
pub(crate) struct AppState {
    store: Arc<Mutex<MemoryStore>>,
    contracts_metadata: Arc<ContractsMetadata>,
}

impl AppState {
    pub(crate) fn new(cfg: Config) -> Self {
        let contracts_metadata = build_contracts_metadata();
        Self {
            store: Arc::new(Mutex::new(MemoryStore {
                runs: HashMap::new(),
                approvals: HashMap::new(),
                audit_last_hash: None,
                audit_path: cfg.audit.jsonl_path,
                audit_mirror_path: cfg.audit.immutable_mirror_path,
            })),
            contracts_metadata: Arc::new(contracts_metadata),
        }
    }

    pub(crate) async fn lock_store(&self) -> tokio::sync::MutexGuard<'_, MemoryStore> {
        self.store.lock().await
    }

    pub(crate) fn contracts_metadata(&self) -> ContractsMetadata {
        (*self.contracts_metadata).clone()
    }
}

#[derive(Default)]
pub(crate) struct MemoryStore {
    runs: HashMap<String, RunEnvelope>,
    approvals: HashMap<String, String>,
    audit_last_hash: Option<String>,
    audit_path: String,
    audit_mirror_path: Option<String>,
}

impl MemoryStore {
    pub(crate) fn insert_run(&mut self, run_id: String, envelope: RunEnvelope) {
        self.runs.insert(run_id, envelope);
    }

    pub(crate) fn get_run(&self, run_id: &str) -> Option<RunEnvelope> {
        self.runs.get(run_id).cloned()
    }

    pub(crate) fn take_run(&mut self, run_id: &str) -> Option<RunEnvelope> {
        self.runs.remove(run_id)
    }

    pub(crate) fn put_run(&mut self, run_id: String, run: RunEnvelope) {
        self.runs.insert(run_id, run);
    }

    pub(crate) fn map_approval_to_run(&mut self, approval_id: String, run_id: String) {
        self.approvals.insert(approval_id, run_id);
    }

    pub(crate) fn run_id_for_approval(&self, approval_id: &str) -> Option<String> {
        self.approvals.get(approval_id).cloned()
    }

    pub(crate) fn run_mut(&mut self, run_id: &str) -> Option<&mut RunEnvelope> {
        self.runs.get_mut(run_id)
    }

    pub(crate) fn append_audit(&mut self, record: AuditRecord) -> Result<(), ApiFailure> {
        let next_hash = append_audit_record(
            &self.audit_path,
            self.audit_mirror_path.as_deref(),
            self.audit_last_hash.as_deref(),
            record,
        )?;
        self.audit_last_hash = Some(next_hash);
        Ok(())
    }
}
