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

## ドキュメント

- `docs/spec/envelopes.md`
- `docs/spec/capability-discovery.md`
- `docs/spec/json-fingerprint.md`
- `docs/spec/governance-view.md`
- `docs/spec/errors.md`
- `docs/releases/v1.2.0.md`
