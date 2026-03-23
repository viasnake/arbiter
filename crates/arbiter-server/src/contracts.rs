use arbiter_contracts::{contracts_manifest_v1, ContractsMetadata, API_VERSION};
use std::collections::BTreeMap;

pub(crate) fn build_contracts_metadata() -> ContractsMetadata {
    let manifest = contracts_manifest_v1();
    let schemas = manifest
        .schemas
        .iter()
        .map(|v| (v.path.to_string(), v.sha256.to_string()))
        .collect::<BTreeMap<_, _>>();

    ContractsMetadata {
        api_version: API_VERSION.to_string(),
        openapi_sha256: manifest.openapi_sha256.to_string(),
        contracts_set_sha256: manifest.contracts_set_sha256.to_string(),
        generated_at: manifest.generated_at.to_string(),
        schemas,
    }
}
