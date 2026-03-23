use arbiter_config::Config;
use arbiter_contracts::{ContractsMetadata, RunEnvelope};
use arbiter_kernel::policy::{ApproverResolverConfig, PolicyConfig};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::audit::{append_audit_record, read_audit_tail_hash, AuditRecord};
use crate::contracts::build_contracts_metadata;
use crate::errors::ApiFailure;

#[derive(Clone)]
pub(crate) struct AppState {
    store: Arc<Mutex<StoreBackend>>,
    contracts_metadata: Arc<ContractsMetadata>,
    policy_config: Arc<PolicyConfig>,
    approver_config: Arc<ApproverResolverConfig>,
    permit_ttl_seconds: u64,
}

impl AppState {
    pub(crate) fn new(cfg: Config) -> Result<Self, String> {
        let contracts_metadata = build_contracts_metadata();
        let last_hash =
            read_audit_tail_hash(&cfg.audit.jsonl_path).map_err(|err| format!("{err:?}"))?;

        let backend = if cfg.store.kind == "sqlite" {
            let sqlite_path = cfg
                .store
                .sqlite_path
                .clone()
                .ok_or_else(|| "sqlite_path is required".to_string())?;
            StoreBackend::Sqlite(SqliteStore::new(
                &sqlite_path,
                cfg.audit.jsonl_path.clone(),
                cfg.audit.immutable_mirror_path.clone(),
                last_hash,
                cfg.governance.idempotency_retention_hours,
            )?)
        } else {
            StoreBackend::Memory(MemoryStore {
                runs: HashMap::new(),
                approvals: HashMap::new(),
                idempotency: HashMap::new(),
                audit_last_hash: last_hash,
                audit_path: cfg.audit.jsonl_path.clone(),
                audit_mirror_path: cfg.audit.immutable_mirror_path.clone(),
                idempotency_retention_hours: cfg.governance.idempotency_retention_hours,
            })
        };

        Ok(Self {
            store: Arc::new(Mutex::new(backend)),
            contracts_metadata: Arc::new(contracts_metadata),
            policy_config: Arc::new(PolicyConfig {
                allowed_providers: cfg.governance.allowed_providers.clone(),
                capability_allowlist: cfg.governance.capability_allowlist.clone(),
                capability_denylist: cfg.governance.capability_denylist.clone(),
                require_approval_for_write_external: cfg.policy.require_approval_for_write_external,
                require_approval_for_notify: cfg.policy.require_approval_for_notify,
                require_approval_for_start_job: cfg.policy.require_approval_for_start_job,
                require_approval_for_production: cfg.policy.require_approval_for_production,
            }),
            approver_config: Arc::new(ApproverResolverConfig {
                default_approvers: cfg.approver.default_approvers,
                production_approvers: cfg.approver.production_approvers,
            }),
            permit_ttl_seconds: cfg.governance.permit_ttl_seconds,
        })
    }

    pub(crate) async fn lock_store(&self) -> tokio::sync::MutexGuard<'_, StoreBackend> {
        self.store.lock().await
    }

    pub(crate) fn contracts_metadata(&self) -> ContractsMetadata {
        (*self.contracts_metadata).clone()
    }

    pub(crate) fn policy_config(&self) -> &PolicyConfig {
        self.policy_config.as_ref()
    }

    pub(crate) fn approver_config(&self) -> &ApproverResolverConfig {
        self.approver_config.as_ref()
    }

    pub(crate) fn permit_ttl_seconds(&self) -> u64 {
        self.permit_ttl_seconds
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IdempotencyRecord {
    pub payload_hash: String,
    pub response_json: String,
    pub created_at: String,
}

pub(crate) enum StoreBackend {
    Memory(MemoryStore),
    Sqlite(SqliteStore),
}

impl StoreBackend {
    pub(crate) fn get_run(&self, run_id: &str) -> Result<Option<RunEnvelope>, ApiFailure> {
        match self {
            StoreBackend::Memory(v) => Ok(v.runs.get(run_id).cloned()),
            StoreBackend::Sqlite(v) => v.get_run(run_id),
        }
    }

    pub(crate) fn put_run(&mut self, run: RunEnvelope) -> Result<(), ApiFailure> {
        match self {
            StoreBackend::Memory(v) => {
                v.runs.insert(run.run.run_id.clone(), run);
                Ok(())
            }
            StoreBackend::Sqlite(v) => v.put_run(run),
        }
    }

    pub(crate) fn find_run_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<RunEnvelope>, ApiFailure> {
        match self {
            StoreBackend::Memory(v) => Ok(v
                .runs
                .values()
                .find(|r| r.run.request_id == request_id)
                .cloned()),
            StoreBackend::Sqlite(v) => v.find_run_by_request_id(request_id),
        }
    }

    pub(crate) fn map_approval_to_run(
        &mut self,
        approval_id: &str,
        run_id: &str,
    ) -> Result<(), ApiFailure> {
        match self {
            StoreBackend::Memory(v) => {
                v.approvals
                    .insert(approval_id.to_string(), run_id.to_string());
                Ok(())
            }
            StoreBackend::Sqlite(v) => v.map_approval_to_run(approval_id, run_id),
        }
    }

    pub(crate) fn run_id_for_approval(
        &self,
        approval_id: &str,
    ) -> Result<Option<String>, ApiFailure> {
        match self {
            StoreBackend::Memory(v) => Ok(v.approvals.get(approval_id).cloned()),
            StoreBackend::Sqlite(v) => v.run_id_for_approval(approval_id),
        }
    }

    pub(crate) fn get_idempotency(
        &mut self,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>, ApiFailure> {
        match self {
            StoreBackend::Memory(v) => {
                if let Some(entry) = v.idempotency.get(key).cloned() {
                    if is_idempotency_expired(&entry.created_at, v.idempotency_retention_hours) {
                        v.idempotency.remove(key);
                        return Ok(None);
                    }
                    return Ok(Some(entry));
                }
                Ok(None)
            }
            StoreBackend::Sqlite(v) => v.get_idempotency(key),
        }
    }

    pub(crate) fn put_idempotency(
        &mut self,
        key: &str,
        payload_hash: &str,
        response_json: &str,
    ) -> Result<(), ApiFailure> {
        match self {
            StoreBackend::Memory(v) => {
                v.idempotency.insert(
                    key.to_string(),
                    IdempotencyRecord {
                        payload_hash: payload_hash.to_string(),
                        response_json: response_json.to_string(),
                        created_at: Utc::now().to_rfc3339(),
                    },
                );
                Ok(())
            }
            StoreBackend::Sqlite(v) => v.put_idempotency(key, payload_hash, response_json),
        }
    }

    pub(crate) fn append_audit(&mut self, record: AuditRecord) -> Result<(), ApiFailure> {
        match self {
            StoreBackend::Memory(v) => {
                let event = append_audit_record(
                    &v.audit_path,
                    v.audit_mirror_path.as_deref(),
                    &v.audit_last_hash,
                    record,
                )?;
                v.audit_last_hash = event.hash;
                Ok(())
            }
            StoreBackend::Sqlite(v) => v.append_audit(record),
        }
    }

    pub(crate) fn audit_path(&self) -> &str {
        match self {
            StoreBackend::Memory(v) => &v.audit_path,
            StoreBackend::Sqlite(v) => &v.audit_path,
        }
    }

    pub(crate) fn doctor(&self) -> Result<Vec<String>, ApiFailure> {
        match self {
            StoreBackend::Memory(v) => Ok(vec![
                "store=memory".to_string(),
                format!("runs={}", v.runs.len()),
                format!("idempotency_records={}", v.idempotency.len()),
            ]),
            StoreBackend::Sqlite(v) => v.doctor(),
        }
    }
}

pub(crate) struct MemoryStore {
    runs: HashMap<String, RunEnvelope>,
    approvals: HashMap<String, String>,
    idempotency: HashMap<String, IdempotencyRecord>,
    audit_last_hash: String,
    audit_path: String,
    audit_mirror_path: Option<String>,
    idempotency_retention_hours: u64,
}

pub(crate) struct SqliteStore {
    conn: Connection,
    audit_last_hash: String,
    audit_path: String,
    audit_mirror_path: Option<String>,
    idempotency_retention_hours: u64,
}

impl SqliteStore {
    fn new(
        sqlite_path: &str,
        audit_path: String,
        audit_mirror_path: Option<String>,
        audit_last_hash: String,
        idempotency_retention_hours: u64,
    ) -> Result<Self, String> {
        let conn = Connection::open(sqlite_path)
            .map_err(|err| format!("failed to open sqlite database: {err}"))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS runs (
                run_id TEXT PRIMARY KEY,
                request_id TEXT UNIQUE NOT NULL,
                envelope_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS approvals (
                approval_id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS idempotency (
                idem_key TEXT PRIMARY KEY,
                payload_hash TEXT NOT NULL,
                response_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            ",
        )
        .map_err(|err| format!("failed to initialize sqlite schema: {err}"))?;
        Ok(Self {
            conn,
            audit_last_hash,
            audit_path,
            audit_mirror_path,
            idempotency_retention_hours,
        })
    }

    fn get_run(&self, run_id: &str) -> Result<Option<RunEnvelope>, ApiFailure> {
        let mut stmt = self
            .conn
            .prepare("SELECT envelope_json FROM runs WHERE run_id = ?1")
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        let row = stmt
            .query_row(params![run_id], |row| row.get::<_, String>(0))
            .ok();
        let Some(text) = row else {
            return Ok(None);
        };
        let run: RunEnvelope =
            serde_json::from_str(&text).map_err(|err| ApiFailure::internal(&err.to_string()))?;
        Ok(Some(run))
    }

    fn put_run(&mut self, run: RunEnvelope) -> Result<(), ApiFailure> {
        let run_id = run.run.run_id.clone();
        let request_id = run.run.request_id.clone();
        let json =
            serde_json::to_string(&run).map_err(|err| ApiFailure::internal(&err.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO runs (run_id, request_id, envelope_json) VALUES (?1, ?2, ?3)
                 ON CONFLICT(run_id) DO UPDATE SET request_id=excluded.request_id, envelope_json=excluded.envelope_json",
                params![run_id, request_id, json],
            )
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        Ok(())
    }

    fn find_run_by_request_id(&self, request_id: &str) -> Result<Option<RunEnvelope>, ApiFailure> {
        let mut stmt = self
            .conn
            .prepare("SELECT envelope_json FROM runs WHERE request_id = ?1")
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        let row = stmt
            .query_row(params![request_id], |row| row.get::<_, String>(0))
            .ok();
        let Some(text) = row else {
            return Ok(None);
        };
        let run: RunEnvelope =
            serde_json::from_str(&text).map_err(|err| ApiFailure::internal(&err.to_string()))?;
        Ok(Some(run))
    }

    fn map_approval_to_run(&mut self, approval_id: &str, run_id: &str) -> Result<(), ApiFailure> {
        self.conn
            .execute(
                "INSERT INTO approvals (approval_id, run_id) VALUES (?1, ?2)
                 ON CONFLICT(approval_id) DO UPDATE SET run_id=excluded.run_id",
                params![approval_id, run_id],
            )
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        Ok(())
    }

    fn run_id_for_approval(&self, approval_id: &str) -> Result<Option<String>, ApiFailure> {
        let mut stmt = self
            .conn
            .prepare("SELECT run_id FROM approvals WHERE approval_id = ?1")
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        let row = stmt
            .query_row(params![approval_id], |row| row.get::<_, String>(0))
            .ok();
        Ok(row)
    }

    fn get_idempotency(&mut self, key: &str) -> Result<Option<IdempotencyRecord>, ApiFailure> {
        self.cleanup_expired_idempotency()?;
        let mut stmt = self.conn.prepare(
            "SELECT payload_hash, response_json, created_at FROM idempotency WHERE idem_key = ?1",
        )
        .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        let row = stmt
            .query_row(params![key], |row| {
                Ok(IdempotencyRecord {
                    payload_hash: row.get(0)?,
                    response_json: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .ok();
        Ok(row)
    }

    fn put_idempotency(
        &mut self,
        key: &str,
        payload_hash: &str,
        response_json: &str,
    ) -> Result<(), ApiFailure> {
        self.cleanup_expired_idempotency()?;
        self.conn
            .execute(
                "INSERT INTO idempotency (idem_key, payload_hash, response_json, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(idem_key) DO NOTHING",
                params![key, payload_hash, response_json, Utc::now().to_rfc3339()],
            )
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        Ok(())
    }

    fn cleanup_expired_idempotency(&mut self) -> Result<(), ApiFailure> {
        let threshold =
            (Utc::now() - Duration::hours(self.idempotency_retention_hours as i64)).to_rfc3339();
        self.conn
            .execute(
                "DELETE FROM idempotency WHERE created_at < ?1",
                params![threshold],
            )
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        Ok(())
    }

    fn append_audit(&mut self, record: AuditRecord) -> Result<(), ApiFailure> {
        let event = append_audit_record(
            &self.audit_path,
            self.audit_mirror_path.as_deref(),
            &self.audit_last_hash,
            record,
        )?;
        self.audit_last_hash = event.hash;
        Ok(())
    }

    fn doctor(&self) -> Result<Vec<String>, ApiFailure> {
        let mut out = vec!["store=sqlite".to_string()];
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM runs")
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        let runs: i64 = stmt
            .query_row([], |row| row.get(0))
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        out.push(format!("runs={runs}"));
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM idempotency")
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        let idem: i64 = stmt
            .query_row([], |row| row.get(0))
            .map_err(|err| ApiFailure::internal(&err.to_string()))?;
        out.push(format!("idempotency_records={idem}"));
        Ok(out)
    }
}

fn is_idempotency_expired(created_at: &str, retention_hours: u64) -> bool {
    let Ok(ts) = DateTime::parse_from_rfc3339(created_at) else {
        return false;
    };
    let threshold = Utc::now() - Duration::hours(retention_hours as i64);
    ts.with_timezone(&Utc) < threshold
}
