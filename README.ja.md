# Arbiter

[日本語](README.ja.md) | [English](README.md)

Arbiter は AI アプリケーション向けの provider-agnostic なガバナンス仲介コンポーネントです。

## 役割

- `/v1/contracts` で契約セットとガバナンス情報を公開
- `ops.event` を決定的に `ops.plan` へ変換
- action type / provider allowlist / approval 方針を強制
- idempotency 衝突を `409 conflict.payload_mismatch` で診断
- 監査ログをハッシュチェーンで保持

## 非スコープ

- Connector / Executor の実装
- 副作用の実行
- プロバイダ固有分岐の実装

## API (v1.2.0)

- `GET /v1/healthz`
- `GET /v1/contracts`
- `POST /v1/events`
- `POST /v1/approval-events`
- `POST /v1/action-results`

OpenAPI: `openapi/v1.yaml`

## 決定性

- `plan.decision.evaluation_time` は `event.occurred_at` 由来
- wall-clock 時刻は plan 決定に使わない

## Docker で実行 (GHCR)

```bash
docker pull ghcr.io/viasnake/arbiter:v1.2.0
docker run --rm -p 8080:8080 \
  -v "$(pwd)/config/example-config.yaml:/app/config/config.yaml:ro" \
  ghcr.io/viasnake/arbiter:v1.2.0 \
  serve --config /app/config/config.yaml
```

リリース時のコンテナタグは `ghcr.io/viasnake/arbiter:vX.Y.Z` のみです（`latest` は付与しません）。

## Schema URL 方針

- JSON schema の `$id` はリリースタグ固定の raw GitHub URL を使用します。
- 例: `https://raw.githubusercontent.com/viasnake/arbiter/v1.2.0/contracts/v1/ops.event.schema.json`
- 新しいリリースでは `$id` を新タグへ更新し、drift guard テストで検証します。

## 検証

```bash
mise run version-check
mise run fmt-check
mise run lint
mise run contracts-verify
mise run test
mise run build
```

## リリース時のバージョン更新

```bash
make version-bump VERSION=1.2.1
mise run ci
```

`version-bump` は Cargo/OpenAPI/API_VERSION/schema `$id`/README の例示バージョンをまとめて更新します。

## ドキュメント

- `docs/spec/envelopes.md`
- `docs/spec/capability-discovery.md`
- `docs/spec/json-fingerprint.md`
- `docs/spec/governance-view.md`
- `docs/spec/errors.md`
- `docs/releases/v1.2.0.md`
