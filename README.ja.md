# Arbiter

[日本語](README.ja.md) | [English](README.md)

Arbiter は、AI 駆動プロダクト向けの決定制御プレーンです。

Arbiter は「次に何をすべきか」を決めます。実行はしません。

## なぜ必要か

多くの AI 事故は、生成品質そのものよりも制御の弱さで発生します。重複実行、不明瞭な認可、見えないリトライ、監査証跡の欠落が原因です。

Arbiter は、決定挙動を明示的・再現可能・診断可能にするために存在します。

生成・ポリシー・実行を 1 つのランタイムで密結合すると、障害後に次の問いへ確実に答えられなくなります。

- なぜこの操作が許可されたのか
- なぜこの分岐が選ばれたのか
- なぜ同じリトライで違う結果になったのか

Arbiter は決定ロジックを実行から分離し、これらを証跡で説明可能にします。

## 典型ユースケース

副作用コストや運用リスクが高いケースで有効です。

- 重複送信を防止したいメッセージング/アシスタント系システム
- 実行前に人間承認が必要なワークフロー
- テナント横断で gate / 認可ポリシーを一貫適用したいシステム
- ジョブ状態とキャンセル整合が必要な長時間エージェント処理
- 障害解析のために決定の再現・監査が必要な運用環境

## Arbiter ができること

- 正規化イベントを契約に基づいて検証する
- gate 判定（cooldown / queue / rate）を行う
- 認可判定と fail posture を適用する
- 決定的な ResponsePlan を生成する
- `(tenant_id, event_id)` の冪等性を保証する
- 追記専用監査ログとハッシュチェーン整合性を維持する
- job / approval ライフサイクルイベントを受けて状態を整合させる

## Arbiter がしないこと（設計上の責務外）

以下は未実装ではなく責務境界です。

- メッセージ送信やツール呼び出しなどの実行
- テキスト生成そのもの
- ジョブワーカーの実行
- エンドユーザー向け承認 UI の提供
- コネクタ用の外部資格情報管理

Arbiter は決定プレーンです。実行は実行プレーンが担います。

## コア保証

- 同一入力・同一ポリシー・同一状態での決定性
- reason code を伴う明示的 fail posture
- イベント処理の冪等性
- 監査ログ内の説明可能な判定トレース
- `prev_hash` / `record_hash` による改ざん検知可能性

## API 概要（v1）

- `POST /v1/events`
- `POST /v1/generations`
- `POST /v1/job-events`
- `POST /v1/job-cancel`
- `POST /v1/approval-events`
- `POST /v1/action-results`
- `GET /v1/action-results/{tenant_id}/{plan_id}/{action_id}`
- `GET /v1/contracts`
- `GET /v1/healthz`

OpenAPI: `openapi/v1.yaml`

`POST /v1/action-results` のリクエスト契約（v1）:

- 必須: `v`, `plan_id`, `action_id`, `tenant_id`, `status`, `ts`
- 任意: `provider_message_id`, `reason_code`, `error`
- `status` 列挙: `succeeded` | `failed` | `skipped`
- 冪等キー: `tenant_id + plan_id + action_id`
- 同一キーでペイロード不一致の場合は `409` を返却（`conflict.payload_mismatch`）

`POST /v1/events` の冪等セマンティクス（v1）:

- 冪等キー: `tenant_id + event_id`
- 同一キーかつ同一ペイロード: 既存の `ResponsePlan` を返却（`200`）
- 同一キーかつ異なるペイロード: `409` を返却（`conflict.payload_mismatch`）

状態参照 API:

- `GET /v1/jobs/{tenant_id}/{job_id}`
- `GET /v1/approvals/{tenant_id}/{approval_id}`
- `GET /v1/action-results/{tenant_id}/{plan_id}/{action_id}`

## Contracts とバージョニング

- 利用中契約セット: `contracts/v1/*`
- 実行時契約バージョン: `v=1`
- OpenAPI のスキーマソース: `openapi/v1.yaml` から `contracts/v1/*` を直接参照
- `GET /v1/contracts` は OpenAPI/contracts 由来の build-time manifest から生成し、ソースハッシュを返却
- 互換性ポリシー: `docs/contract-compatibility-policy.md`

## ストレージ

サポートされる store:

- `memory`
- `sqlite`

上記以外の `store.kind` は起動時に失敗します。
`store.kind=sqlite` の場合、`store.sqlite_path` は必須です。

SQLite マイグレーション基準:

- 起動時に `CREATE TABLE IF NOT EXISTS` で不足テーブルを作成
- 進化は additive-first
- アップグレードで決定性と冪等性の意味を壊さない

## 監査整合性

監査レコードは追記専用で、ハッシュチェーン整合性のためのフィールドを持ちます。

- `prev_hash`: 直前レコードのハッシュ
- `record_hash`: 現在レコード seed のハッシュ

`audit.immutable_mirror_path` により、オプションの immutable mirror sink を設定できます。

監査チェーン検証:

```bash
arbiter audit-verify --path ./arbiter-audit.jsonl
arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl
```

## クイックスタート

ツールチェーンを準備:

```bash
mise install
```

サーバ起動:

```bash
mise exec -- cargo run -- serve --config ./config/example-config.yaml
```

バイナリビルド:

```bash
mise run build
./target/release/arbiter serve --config ./config/example-config.yaml
```

## ローカル品質ゲート

CI 相当チェック:

```bash
mise run fmt-check
mise run lint
mise run contracts-verify
mise run test
mise run build
mise run ci
```

## リリース自動化

- SemVer タグ（例: `v1.1.0`）を push すると release workflow が起動します。
- 公開前に tag/Cargo/OpenAPI のバージョン整合を検証します。
- GitHub Release notes は自動生成され、バイナリと checksum を添付します。
- 変更履歴とリリーススコープ: `CHANGELOG.md`, `docs/releases/v1.1.0.md`

## 運用

- SLO: `docs/slo.md`
- Runbook: `docs/runbook.md`
- AuthZ 耐障害ポリシー: `docs/authz-resilience.md`
- 状態セマンティクス: `docs/state/events.md`, `docs/state/jobs.md`, `docs/state/approvals.md`, `docs/state/action-results.md`
- 監査 mirror セマンティクス: `docs/audit-mirror.md`
- Contracts メタデータ生成: `docs/contracts-metadata.md`
- Contracts endpoint セマンティクス: `docs/contracts-endpoint.md`
- リリーススコープ（v1.1.0）: `docs/releases/v1.1.0.md`

## 設計ドキュメント

- `docs/architecture-principles.md`
- `docs/decision-log.md`
- `docs/operational-philosophy.md`
- `docs/extensibility-roadmap.md`
- `docs/contracts-intent.md`
- `docs/contract-compatibility-policy.md`
