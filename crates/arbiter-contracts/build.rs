use chrono::{SecondsFormat, Utc};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let contracts_dir = repo_root.join("contracts/v1");
    let openapi_path = repo_root.join("openapi/v1.yaml");

    println!("cargo:rerun-if-changed={}", contracts_dir.display());
    println!("cargo:rerun-if-changed={}", openapi_path.display());

    let mut schema_paths: Vec<PathBuf> = fs::read_dir(&contracts_dir)
        .expect("read contracts/v1")
        .filter_map(|entry| entry.ok().map(|v| v.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".schema.json"))
                .unwrap_or(false)
        })
        .collect();
    schema_paths.sort();

    let mut contracts_set_hasher = Sha256::new();
    let mut rows = Vec::new();

    for path in schema_paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes =
            fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let canonical_path = canonical_contract_ref(&repo_root, &path);
        let schema_sha = hex_sha256(&bytes);

        contracts_set_hasher.update(canonical_path.as_bytes());
        contracts_set_hasher.update([0]);
        contracts_set_hasher.update(&bytes);
        contracts_set_hasher.update([0]);

        let body = String::from_utf8(bytes)
            .unwrap_or_else(|e| panic!("schema is not valid utf-8 {}: {e}", path.display()));
        rows.push((canonical_path, schema_sha, body));
    }

    let openapi_bytes = fs::read(&openapi_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", openapi_path.display()));
    let openapi_sha = hex_sha256(&openapi_bytes);
    let contracts_set_digest = contracts_set_hasher.finalize();
    let contracts_set_sha: String = contracts_set_digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    let mut out = String::new();
    out.push_str("pub const GENERATED_OPENAPI_SHA256: &str = ");
    out.push_str(&format!("{openapi_sha:?};\n"));
    out.push_str("pub const GENERATED_CONTRACTS_SET_SHA256: &str = ");
    out.push_str(&format!("{contracts_set_sha:?};\n"));
    out.push_str("pub const GENERATED_AT_RFC3339: &str = ");
    out.push_str(&format!("{generated_at:?};\n"));
    out.push_str("pub const GENERATED_CONTRACT_SCHEMAS: &[(&str, &str, &str)] = &[\n");
    for (path, sha, body) in rows {
        out.push_str("    (");
        out.push_str(&format!("{path:?}, {sha:?}, {body:?}"));
        out.push_str("),\n");
    }
    out.push_str("];\n");

    let out_path = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_path.join("generated_contracts.rs"), out).expect("write generated_contracts.rs");
}

fn canonical_contract_ref(repo_root: &Path, full_path: &Path) -> String {
    let rel = full_path.strip_prefix(repo_root).unwrap_or_else(|e| {
        panic!(
            "failed to strip repo root from {}: {e}",
            full_path.display()
        )
    });
    format!("../{}", rel.to_string_lossy().replace('\\', "/"))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
