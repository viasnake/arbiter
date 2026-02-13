# RFC-ARB-0001

# Arbiter v0.0.1 仕様書

Status: Draft
Version: v0.0.1
Date: 2026-02-13

---

# 1. 目的（Purpose）

Arbiter は、AI システムにおける **決定論的制御プレーン**である。

Arbiter は以下を行う：

* Event を受け取る
* 決定論的に ResponsePlan を生成する
* 認可（RBAC）を適用する
* Gate（負荷制御）を適用する
* 監査ログを記録する
* 冪等性を保証する

Arbiter は生成・実行を行わない。

---

# 2. 非目的（Non-Goals）

Arbiter は以下を提供しない：

* LLM テキスト生成
* プロンプト管理
* Agent ループ
* ツール実行
* Adapter 実装
* Persona 管理
* データ検索（RAG）

---

# 3. 設計原則

1. 決定論（Determinism）
2. 分離（Separation of Concerns）
3. 単一責務（Control Plane Only）
4. プロジェクト非依存
5. 単一バイナリ
6. 設定駆動

---

# 4. システム概要

```
Adapter → Arbiter → Generator Runner
                    ↓
               Agent Runtime
```

Arbiter は中央に位置し、行動の裁定のみを行う。

---

# 5. 中核関数

Arbiter の本質は以下の関数である：

```
process(Event) -> ResponsePlan
```

---

# 6. データ契約（Contracts）

## 6.1 Event

Event は外部からの正規化入力である。

必須フィールド：

* v
* event_id
* tenant_id
* source
* room_id
* actor
* content
* ts

Event は JSON Schema v0 に準拠 MUST。

---

## 6.2 ResponsePlan

ResponsePlan は次に行うべき Action 群である。

必須フィールド：

* v
* plan_id
* tenant_id
* room_id
* actions

ResponsePlan は決定論的でなければならない。

---

## 6.3 Action Types（v0.0.1）

Arbiter v0.0.1 で有効な Action は以下のみ：

* do_nothing
* request_generation
* send_message
* send_reply

以下は予約のみ（未実装）：

* start_agent_job
* request_approval

---

# 7. 処理パイプライン

Event 受信時、以下の順序で処理 MUST。

1. Schema Validation
2. Idempotency Check
3. Load RoomState
4. Gate Evaluation
5. Authorization (AuthZ)
6. Planner Evaluation
7. ResponsePlan Emit
8. Audit Log Persist

順序変更は禁止。

---

# 8. 冪等性

同一 tenant_id + event_id の組み合わせに対し：

* 同一の ResponsePlan を返す MUST
* Action type と target は一致 MUST

---

# 9. Gate 仕様

Gate 判定順序 MUST：

1. generating lock
2. cooldown
3. backpressure
4. tenant-level rate limit

違反時：

* do_nothing を返す MUST
* reason_code を記録 MUST

---

# 10. Planner 仕様

Planner は LLM 非依存。

Intent は以下のみ：

* IGNORE
* REPLY
* MESSAGE

確率判定は：

```
seed = hash(event_id)
```

を使用し、決定論的に行う MUST。

---

# 11. 認可（Authorization）

Arbiter v0.0.1 は以下の認可モードを持つ：

* builtin
* external_http

## 11.1 external_http

Arbiter は AuthZRequest を外部プロバイダへ送信する。

AuthZDecision を受信し、以下を適用 MUST：

* decision=deny → do_nothing
* decision=allow → 継続

fail_mode 設定に従う。

---

# 12. 監査ログ（Audit）

監査は append-only MUST。

最低限以下を記録：

* audit_id
* tenant_id
* correlation_id
* action
* result
* reason_code
* ts

v0.0.1 では hash_chain は必須ではない。

---

# 13. HTTP API

| Method | Path               | 説明          |
| ------ | ------------------ | ----------- |
| POST   | /v0/events         | Event 受付    |
| POST   | /v0/generations    | 生成結果受付      |
| POST   | /v0/action-results | 実行結果受付      |
| GET    | /v0/contracts      | Action 定義取得 |
| GET    | /v0/healthz        | ヘルスチェック     |

---

# 14. 設定

Arbiter は config.yaml で動作を決定する。

例：

```yaml
store:
  type: memory

authz:
  mode: external_http
  endpoint: http://authz:8081/v0/authorize
  fail_mode: deny

gate:
  cooldown_ms: 3000
  max_queue: 10

planner:
  reply_policy: mention_first
  reply_probability: 0.3
```

プロジェクト差分は config で吸収 MUST。

---

# 15. 単一バイナリ要件

Arbiter MUST:

* 単一実行バイナリである
* `arbiter serve` で起動可能
* 依存外部コンポーネントを持たない

---

# 16. 決定論保証

以下が同一である場合：

* Event
* RoomState
* Policy
* AuthZDecision

ResponsePlan は完全一致 MUST。

---

# 17. セキュリティ

* fail_mode はデフォルト deny 推奨
* 認可失敗時は deny
* AuthZ タイムアウトを制御
* 監査ログは改ざん不可（将来拡張）

---

# 18. v0.0.1 完了基準

* request_generation が返る
* do_nothing が返る
* 外部RBAC動作
* JSONL監査出力
* 冪等性テスト通過
* 単一バイナリ生成

---

# 19. 将来拡張（v0.1+）

* Job lifecycle
* Capability grant
* Approval workflow
* Audit hash chain
* Multi-region state

---

# 20. 結論

Arbiter v0.0.1 は：

> LLM や Agent を安全に動かすための裁定装置である。

生成も実行も行わない。
