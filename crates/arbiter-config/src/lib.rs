use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("read config failed: {0}")]
    Read(String),
    #[error("parse config failed: {0}")]
    Parse(String),
    #[error("schema load failed: {0}")]
    SchemaLoad(String),
    #[error("schema validation failed: {0}")]
    SchemaValidation(String),
    #[error("unsupported config: {0}")]
    UnsupportedConfig(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: Server,
    pub store: Store,
    pub governance: Governance,
    pub policy: Policy,
    pub audit: Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub listen_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    pub kind: String,
    pub sqlite_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Governance {
    pub allowed_providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub version: String,
    #[serde(default = "default_require_write_external")]
    pub require_approval_for_write_external: bool,
    #[serde(default = "default_require_notify")]
    pub require_approval_for_notify: bool,
    #[serde(default = "default_require_start_job")]
    pub require_approval_for_start_job: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Audit {
    pub jsonl_path: String,
    #[serde(default)]
    pub immutable_mirror_path: Option<String>,
}

fn default_require_write_external() -> bool {
    true
}

fn default_require_notify() -> bool {
    false
}

fn default_require_start_job() -> bool {
    false
}

pub fn load_and_validate(path: &str) -> Result<Config, ConfigError> {
    let config_text =
        std::fs::read_to_string(path).map_err(|err| ConfigError::Read(err.to_string()))?;
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&config_text).map_err(|err| ConfigError::Parse(err.to_string()))?;
    let json_value =
        serde_json::to_value(yaml).map_err(|err| ConfigError::Parse(err.to_string()))?;

    validate_against_schema(&json_value)?;

    let cfg: Config =
        serde_json::from_value(json_value).map_err(|err| ConfigError::Parse(err.to_string()))?;
    validate_runtime_support(&cfg)?;
    Ok(cfg)
}

fn validate_against_schema(instance: &serde_json::Value) -> Result<(), ConfigError> {
    let schema_path = [
        std::path::PathBuf::from("config/config.schema.json"),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("config/config.schema.json"),
    ]
    .into_iter()
    .find(|path| path.exists())
    .ok_or_else(|| {
        ConfigError::SchemaLoad("config schema not found at config/config.schema.json".to_string())
    })?;

    let schema_text = std::fs::read_to_string(schema_path)
        .map_err(|err| ConfigError::SchemaLoad(err.to_string()))?;
    let schema: serde_json::Value = serde_json::from_str(&schema_text)
        .map_err(|err| ConfigError::SchemaLoad(err.to_string()))?;

    let validator = jsonschema::validator_for(&schema)
        .map_err(|err| ConfigError::SchemaLoad(err.to_string()))?;
    if let Err(first) = validator.validate(instance) {
        return Err(ConfigError::SchemaValidation(first.to_string()));
    }
    Ok(())
}

fn validate_runtime_support(cfg: &Config) -> Result<(), ConfigError> {
    if cfg.store.kind != "memory" && cfg.store.kind != "sqlite" {
        return Err(ConfigError::UnsupportedConfig(
            "config.invalid_store_kind: store.kind must be memory|sqlite".to_string(),
        ));
    }

    if cfg.store.kind == "sqlite"
        && cfg
            .store
            .sqlite_path
            .as_ref()
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(ConfigError::UnsupportedConfig(
            "store.sqlite_path is required when store.kind=sqlite".to_string(),
        ));
    }

    if cfg.governance.allowed_providers.is_empty() {
        return Err(ConfigError::UnsupportedConfig(
            "governance.allowed_providers must not be empty".to_string(),
        ));
    }

    if cfg.policy.version.trim().is_empty() {
        return Err(ConfigError::UnsupportedConfig(
            "policy.version must not be empty".to_string(),
        ));
    }

    Ok(())
}
