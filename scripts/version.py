#!/usr/bin/env python3

import argparse
import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]

CARGO_FILES = [
    "Cargo.toml",
    "crates/arbiter-config/Cargo.toml",
    "crates/arbiter-contracts/Cargo.toml",
    "crates/arbiter-kernel/Cargo.toml",
    "crates/arbiter-server/Cargo.toml",
]

SCHEMA_FILES = [
    "contracts/v1/ops.event.schema.json",
    "contracts/v1/ops.action.schema.json",
    "contracts/v1/ops.plan.schema.json",
    "contracts/v1/ops.approval_event.schema.json",
    "contracts/v1/ops.action_result.schema.json",
    "contracts/v1/ops.errors.schema.json",
    "contracts/v1/ops.capabilities.schema.json",
    "contracts/v1/ops.contracts_metadata.schema.json",
    "config/config.schema.json",
]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def ensure_semver(version: str) -> None:
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"invalid version: {version} (expected X.Y.Z)")


def root_version() -> str:
    cargo = read("Cargo.toml")
    m = re.search(r'^version = "([0-9]+\.[0-9]+\.[0-9]+)"$', cargo, re.MULTILINE)
    if not m:
        raise RuntimeError("failed to find root Cargo.toml package.version")
    return m.group(1)


def replace_or_die(path: str, pattern: str, replacement: str, expected_count: int = 1) -> bool:
    text = read(path)
    updated, count = re.subn(pattern, replacement, text, flags=re.MULTILINE)
    if count != expected_count:
        raise RuntimeError(
            f"{path}: replacement count mismatch for pattern {pattern!r} (expected {expected_count}, got {count})"
        )
    if updated != text:
        write(path, updated)
        return True
    return False


def expected_schema_id(version: str, rel_path: str) -> str:
    return f"https://raw.githubusercontent.com/viasnake/arbiter/v{version}/{rel_path}"


def bump(version: str) -> int:
    ensure_semver(version)
    changed = False

    for path in CARGO_FILES:
        changed |= replace_or_die(path, r'^version = "[0-9]+\.[0-9]+\.[0-9]+"$', f'version = "{version}"')

    changed |= replace_or_die(
        "openapi/v1.yaml",
        r"^  version: [0-9]+\.[0-9]+\.[0-9]+$",
        f"  version: {version}",
    )

    changed |= replace_or_die(
        "crates/arbiter-contracts/src/lib.rs",
        r'^pub const API_VERSION: &str = "[0-9]+\.[0-9]+\.[0-9]+";$',
        f'pub const API_VERSION: &str = "{version}";',
    )

    for path in SCHEMA_FILES:
        schema_id = expected_schema_id(version, path)
        changed |= replace_or_die(
            path,
            r'^(\s*"\$id":\s*")[^"]+(",?)$',
            f'\\1{schema_id}\\2',
        )

    changed |= replace_or_die(
        "config/config.schema.json",
        r'^  "title": "Arbiter Configuration Schema v[0-9]+\.[0-9]+\.[0-9]+",$',
        f'  "title": "Arbiter Configuration Schema v{version}",',
    )

    changed |= replace_or_die(
        "contracts/v1/ops.contracts_metadata.schema.json",
        r'^    "api_version": \{ "type": "string", "const": "[0-9]+\.[0-9]+\.[0-9]+" \},$',
        f'    "api_version": {{ "type": "string", "const": "{version}" }},',
    )

    changed |= replace_or_die(
        "README.md",
        r"^## API surface \(v[0-9]+\.[0-9]+\.[0-9]+\)$",
        f"## API surface (v{version})",
    )
    changed |= replace_or_die(
        "README.ja.md",
        r"^## API \(v[0-9]+\.[0-9]+\.[0-9]+\)$",
        f"## API (v{version})",
    )

    for path in ["README.md", "README.ja.md"]:
        changed |= replace_or_die(
            path,
            r"^(docker pull ghcr\.io/viasnake/arbiter:)v[0-9]+\.[0-9]+\.[0-9]+$",
            f"\\1v{version}",
        )
        changed |= replace_or_die(
            path,
            r"^(\s+ghcr\.io/viasnake/arbiter:)v[0-9]+\.[0-9]+\.[0-9]+(\s+\\)?$",
            f"\\1v{version}\\2",
        )
        changed |= replace_or_die(
            path,
            r"(https://raw\.githubusercontent\.com/viasnake/arbiter/)v[0-9]+\.[0-9]+\.[0-9]+(/contracts/v1/ops\.event\.schema\.json)",
            f"\\1v{version}\\2",
        )
        changed |= replace_or_die(
            path,
            r"(`docs/releases/)v[0-9]+\.[0-9]+\.[0-9]+(\.md`)",
            f"\\1v{version}\\2",
        )

    changed |= replace_or_die(
        "README.md",
        r'("policy_version": "policy:)v[0-9]+\.[0-9]+\.[0-9]+(",)',
        f'\\1v{version}\\2',
    )
    changed |= replace_or_die(
        "config/example-config.yaml",
        r'^(  version: "policy:)v[0-9]+\.[0-9]+\.[0-9]+(")$',
        f'\\1v{version}\\2',
    )

    if changed:
        print(f"updated version references to {version}")
    else:
        print(f"no changes needed for {version}")
    return 0


def check() -> int:
    version = root_version()
    failures = []

    for path in CARGO_FILES:
        text = read(path)
        m = re.search(r'^version = "([0-9]+\.[0-9]+\.[0-9]+)"$', text, re.MULTILINE)
        if not m or m.group(1) != version:
            failures.append(f"{path}: expected version {version}")

    openapi = read("openapi/v1.yaml")
    if not re.search(rf"^  version: {re.escape(version)}$", openapi, re.MULTILINE):
        failures.append("openapi/v1.yaml: info.version mismatch")

    lib = read("crates/arbiter-contracts/src/lib.rs")
    if not re.search(rf'^pub const API_VERSION: &str = "{re.escape(version)}";$', lib, re.MULTILINE):
        failures.append("crates/arbiter-contracts/src/lib.rs: API_VERSION mismatch")

    for path in SCHEMA_FILES:
        expected = expected_schema_id(version, path)
        text = read(path)
        m = re.search(r'^\s*"\$id":\s*"([^"]+)"', text, re.MULTILINE)
        if not m or m.group(1) != expected:
            failures.append(f"{path}: $id mismatch (expected {expected})")

    config_schema = read("config/config.schema.json")
    if not re.search(
        rf'^  "title": "Arbiter Configuration Schema v{re.escape(version)}",$',
        config_schema,
        re.MULTILINE,
    ):
        failures.append("config/config.schema.json: title version mismatch")

    metadata_schema = read("contracts/v1/ops.contracts_metadata.schema.json")
    if not re.search(
        rf'^    "api_version": \{{ "type": "string", "const": "{re.escape(version)}" \}},$',
        metadata_schema,
        re.MULTILINE,
    ):
        failures.append("contracts/v1/ops.contracts_metadata.schema.json: api_version const mismatch")

    readme_checks = [
        ("README.md", rf"^## API surface \(v{re.escape(version)}\)$"),
        ("README.ja.md", rf"^## API \(v{re.escape(version)}\)$"),
        ("README.md", rf"^docker pull ghcr\.io/viasnake/arbiter:v{re.escape(version)}$"),
        ("README.ja.md", rf"^docker pull ghcr\.io/viasnake/arbiter:v{re.escape(version)}$"),
        (
            "README.md",
            rf"https://raw\.githubusercontent\.com/viasnake/arbiter/v{re.escape(version)}/contracts/v1/ops\.event\.schema\.json",
        ),
        (
            "README.ja.md",
            rf"https://raw\.githubusercontent\.com/viasnake/arbiter/v{re.escape(version)}/contracts/v1/ops\.event\.schema\.json",
        ),
        ("README.md", rf"`docs/releases/v{re.escape(version)}\.md`"),
        ("README.ja.md", rf"`docs/releases/v{re.escape(version)}\.md`"),
        ("README.md", rf'"policy_version": "policy:v{re.escape(version)}",'),
    ]
    for path, pattern in readme_checks:
        if not re.search(pattern, read(path), re.MULTILINE):
            failures.append(f"{path}: missing expected version reference ({pattern})")

    config_example = read("config/example-config.yaml")
    if not re.search(rf'^  version: "policy:v{re.escape(version)}"$', config_example, re.MULTILINE):
        failures.append("config/example-config.yaml: policy version mismatch")

    if failures:
        print("version check failed:")
        for f in failures:
            print(f"- {f}")
        return 1

    print(f"version check passed: {version}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Arbiter release version automation")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("check", help="verify all version references are consistent")
    p_bump = sub.add_parser("bump", help="update all version references to X.Y.Z")
    p_bump.add_argument("version", help="target version (X.Y.Z)")

    args = parser.parse_args()
    if args.command == "check":
        return check()
    if args.command == "bump":
        return bump(args.version)
    return 2


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:  # pragma: no cover
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
