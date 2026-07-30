# CaseGraphen 独立化と実行制御 — 調査結果と設計

作成日: 2026-07-30
対象: `CAPHTECH/higher-graphen` の `tools/casegraphen` を本リポジトリ (`CAPHTECH/casegraphen`) へ切り出し、実行制御を追加する設計。
調査対象: HigherGraphen v0.7.1 ワークスペース実体（コード・Cargo依存・スキーマ・仕様文書・テスト）、および参照資料としての `CAPHTECH/casegraphen-legacy`。

---

## 1. 調査結果（事実）

### 1.1 リポジトリ状況

- 本リポジトリは調査時点で空（コミットなし）。旧独立実装は GitHub 上で `CAPHTECH/casegraphen-legacy` にリネーム済みで、`casegraphen` の名前は切り出し先として確保されている。
- HigherGraphen の全 crate は crates.io に公開済み（`cargo install casegraphen` が案内されている。v0.7.1 タグは公開が明示承認制のため未公開の可能性あり）。切り出し後は公開版 `higher-graphen-*` crate への依存が可能。

### 1.2 tools/casegraphen の実装インベントリ

約 24.6 KLOC、124 テスト（unit 81 + 実バイナリ経由の統合 43）。**3世代のモデルが同居**している。

| 世代 | モデル | ストア | CLI |
|---|---|---|---|
| legacy case graph | `model.rs` | `store.rs`（上書き型フラットファイル） | `create/inspect/coverage/...`（正典契約から除外済みの移行エイリアス） |
| workflow graph | `workflow_model.rs` | `workflow_workspace`（append-only history.jsonl + revision snapshot） | `workflow *`, `cg workflow *` |
| native case space | `native_model.rs` | `native_store`（append-only morphism_log.jsonl + replay checksum） | `space/case/lift/morphism/invariant/...`（正典サーフェス） |

依存は `higher-graphen-core / structure / reasoning / projection` の4つ。ただし:

- `higher-graphen-projection` は **宣言のみで未使用**（削除可能）。
- `higher-graphen-reasoning` の利用は `math_diagnostics.rs` の有界時相検査2種のみ。
- 非公開API依存ゼロ、`highergraphen-cli` とのコード共有ゼロ。**crate境界での切り出しはクリーン**。

### 1.3 実行系の不在（確定）

grep により確認: `worker / dispatch / scheduler / queue / lease / execute` に相当する概念はソース中に存在しない。具体的には:

- `WorkItem` に worker / assignee / lease フィールドがない。`WorkItemState` に in-flight 系の状態がない。
- `WorkItemState` を書き換えるコードが1行もない（状態変更はグラフJSONを手編集して再importするしかない）。
- `TransitionType::StateTransition` は定義のみで一度も構築されない。
- native store の `apply_bounded_morphism` は「typed reducers が存在するまでメタデータのみの morphism を受理する」と明示（`native_store.rs:519-536`）。**構造変更を実現する reducer が最大の実装ギャップ**。
- workflow 側の patch は `applied: false` / `materialized_record_count: 0` が固定値で、記録のみ・適用なし。
- `review accept|reject|reopen|waive` コマンドは仕様のみで未実装（review はメタデータのみの morphism として表現）。
- `MorphismLogEntry.previous_entry_hash` はスキーマにあるが常に `None`（ハッシュ連鎖は未稼働）。

一方、実行制御に必要な**語彙と検証機構は既にある**: readiness の再導出（「readiness は保存された事実ではなく射影である」が規範）、morphism propose→check→apply→reject、`--base-revision-id` による stale-base 検査、replay checksum、close invariant 12種、`NativeOperationGate`（actor/capability/policy/source_boundary）、`EvidenceRecord`（`evidence_origin: source_backed|inferred|review_promoted`、`EvidenceType::CommandOutput/TestResult/TransitionWitness`）、projection loss。

### 1.4 契約上の制約（仕様文書より）

1. **「外部システムに対するシナリオ実行」は明示的な非ゴール**（`casegraphen.md:537`）。worker / dispatch / execution plan という語は仕様9文書のどこにも登場しない。実行制御は「文書化済みだが未実装」ではなく、**契約境界の意図的改定を伴う新設**である。
2. **「workflow 固有コードは `higher-graphen-runtime`・外部 CaseGraphen リポジトリに依存してはならない」**（`casegraphen-feature-completion-contract.md:316-318`）。依存方向 `casegraphen -> higher-graphen crates` は仕様側で既に確定している。
3. `casegraphen-native-case-management.md:48` は「外部の CaseGraphen リポジトリ（本リポジトリ）を変更しないこと」を非ゴールに挙げており、本切り出しはこの境界の意図的な反転にあたる。→ HigherGraphen 側での ADR が必要。
4. AI 生成構造は常に `unreviewed` で生まれ、明示的な review 行為なしに受理事実へ昇格してはならない（typed provenance の `Reviewed<T, Candidate|Accepted>` typestate は「永続化された accepted 値もデシリアライズ時は candidate に戻る」まで強制する）。
5. 命名規則: intermediate tool は素の小文字 `casegraphen`（`higher-graphen` プレフィックス禁止）。
6. `COMMERCIAL_BOUNDARY.md` は「ホスト型実行基盤・運用 runbook・認証情報」を公開リポジトリ外に置くべきとしており、**実行制御を持つ CaseGraphen の分離はむしろ境界文書と整合する**。

### 1.5 higher-graphen-runtime との責務重複 — 存在しない

runtime は純粋・同期・副作用なしのレポート生成ライブラリ（`run_*(input) -> RuntimeResult<Report>` が9本、`std::fs`/`std::process`/async の使用ゼロ、store/queue/worker なし）。仕様上も「永続ストレージ・スナップショット・リモートサービス・intermediate tool のコマンド実装」を明示的に対象外とする。runtime の "execution" はエンジン操作（query/transform/project/review）の実行を指し、プロセス実行や dispatch を意味しない。

公認の連携形はファイル/スキーマレベルの一方向のみ: runtime が casegraphen の `highergraphen.case.space.v1` JSON を読む（`highergraphen ddd input from-case-space`）。逆方向にしたい場合も **runtime のレポート JSON を証拠入力として消費する**（crate 依存にしない）のが両方の条項を満たす形。

副作用を伴う外部プロセス実行の先例は CLI 層に既にある: `highergraphen-cli` の `semantic_proof_backend.rs` が外部バイナリを起動し、stdout/stderr をハッシュ化して `trust_boundary: local_process_output_untrusted_until_..._verify_and_review` として記録する。**Worker dispatch の信頼境界の雛形**。

### 1.6 下位 crate が提供するもの / しないもの（実行ループ視点）

| ループ工程 | 既存機構 | ギャップ |
|---|---|---|
| 計画+revision固定 | `Revision` + `base_revision` stale検査 + replay checksum（tool側） | crate側に revision/transaction/commit 概念ゼロ。**ExecutionPlan レコードは未存在** |
| readiness 再導出 | `evaluate_readiness` / `frontier_cell_ids`（tool側）。「readiness は射影」が規範 | なし（直接再利用可） |
| WorkItem 選択 | frontier は集合として導出済み | 選択ポリシー/順序の概念なし |
| capability/policy 検査 | `Capability`（preconditions/postconditions/validity_interval, `CapabilityOperation::ExecuteMorphism` あり）、`Policy`、`NativeOperationGate` | gate の配線は close-check のみ。**authorization 決定関数が crate に存在しない** |
| Worker への射影渡し | `Projection` + `ProjectionSelector` + 必須 `InformationLoss` + `measure_projection_loss` | Worker 抽象・transport・ingest 境界が皆無 |
| 出力/証拠/遷移候補の受領 | `EvidenceRecord`（origin別）、`Witness`/`Derivation`、`CompletionCandidate` | **証拠の書き込みパスがない**（EvidenceRecord を append するコードが存在しない） |
| 遷移の morphism 検証 | `check_morphism`、`TransitionRecord`（precondition/postcondition/preservation_checks）、`compose_morphisms_checked`、pullback/pushout | `Morphism` に apply がない。`check_preservation` は宣言の集合照合であり状態から再導出しない（＝EvaluatorKernel で前後検査する必要） |
| 不変条件再検査 | `EvaluatorKernel` + `CheckInput::changed_cells`（増分）+ `CheckResult::to_obstruction` | `EvaluatorCheck` は閉じた enum（6種、カスタム述語不可）。「成功条件」の型がない |
| 新 revision 記録 | `append_morphism`（sequence 単調・stale拒否・checksum照合） | **typed reducers 不在によりメタデータのみ**。`previous_entry_hash` 未populated |
| 履歴・射影損失 | `MorphismLogEntry`/`TransitionTraceRecord`/projection loss 語彙一式 | 損失側は完備。履歴は tool 側実装のみ |

### 1.7 legacy（旧独立実装）からの運用知見

- Worker protocol の**データ契約**までは定義し、実効果を伴う worker（shell / local LLM / code-agent）実行は「外部プロセス実行・状態外ファイル書込・ネットワーク資格情報を扱うため、**別途セキュリティと承認ポリシーの検討を要する**」として意図的に保留した。
- `casegraphen_generalized_design.md`（HigherGraphen 構想の源流）のガードレール: 「AI 生成の Inference を Evidence/Observation として保存してはならない」「既存の readiness 意味論を暗黙に変えない」。

### 1.8 切り出し時の物理的結合

- `include_str!` による `../../../schemas/casegraphen/*.example.json` 参照が約14箇所（src 8 + tests 4 + 統合テストの実行時パス解決）。
- 統合テストが `python3 -m jsonschema` をサブプロセス起動（JSON Schema 検証 crate は不使用。スキーマIDはRust側定数に手動複製されておりドリフト可能）。
- レポート内 `"tool_package": "casegraphen"` がスキーマ検証対象の値。
- Cargo.toml は version/license/lints/依存すべて workspace 継承。
- HigherGraphen 側: `examples/architecture` が `casegraphen` への **path 依存**を持つ。`scripts/check-static-limits.py` / `validate-json-contracts.py` が casegraphen を対象に含む。

---

## 2. 設計判断

各判断は ADR として新リポジトリの `docs/adr/` に固定する前提。

**D1. 依存方向**: `casegraphen -> 公開版 higher-graphen-{core, structure, reasoning}` のみ。`higher-graphen-runtime` へは依存しない（仕様の禁止条項に従う）。runtime のレポートが必要な場合は**レポート JSON を証拠入力として消費**する。`higher-graphen-projection` 依存は削除（未使用）。`higher-graphen-evidence` は確信度の定量評価が必要になるまで追加しない。HigherGraphen が自身の開発管理にリリース済み `casegraphen` CLI を使うのは可（バイナリ利用であり Cargo 依存・リリース循環を作らない）。

**D2. 実行基盤は native case space 世代**。理由: (a) 正典契約（`casegraphen.md`）の中心であり、morphism log / revision / replay checksum / close gate / capability gate が揃っている、(b) HigherGraphen 自身のロードマップ（「Step 8 Planned: full review commands, arbitrary typed morphism reducers, native case close」）と一致する、(c) workflow graph は readiness 推論の wire sidecar として維持し、実行状態の真実は morphism log に一本化する（導出状態の二重管理を作らない）。

**D3. legacy 世代と死コードは移設時に削除**。legacy case graph 群（`model.rs`/`eval.rs`/`report.rs`/`LocalCaseStore`、約1,720行）は正典契約が「移行エイリアス」と宣言済みであり、新リポジトリには持ち込まない。`native_report.rs`（自テスト以外から未参照）も削除。互換性維持は要求されていない。

**D4. スキーマは tool に帰属し新リポジトリへ移管**。`schemas/casegraphen/**`（20ファイル）と統合テストが要する参照 fixture を本リポジトリへ移す。runtime の `ddd input from-case-space` はスキーマファイルを実行時に読まない（手書きパーサ）ため HigherGraphen のビルドに影響しない。`examples/casegraphen/**` は HigherGraphen の example として残す。

**D5. 非ゴール改定は HigherGraphen 側 ADR で明示**。「Executing scenarios against external systems は非ゴール」(`casegraphen.md:537`) と「外部 casegraphen リポジトリを変更しない」(`native-case-management.md:48`) の2条項の改定・失効を silent drift にせず記録する。切り出し後、HigherGraphen 内の casegraphen 仕様文書は本リポジトリへ移管し、HigherGraphen 側にはポインタを残す。

**D6. 信頼モデル — 受理済み計画による事前承認 + 決定論ゲート**。「A patch transition is not applied merely because it is valid; application requires an explicit command or review workflow」という既存規範を保ったまま実行を可能にするため:

- **ExecutionPlan 自体が review 対象**。plan は candidate として生まれ、明示的な review morphism（reviewer_id + reason）で accepted になって初めて実行可能になる。
- accepted plan は「どの WorkItem を、どの Worker binding で、どの遷移クラス（transition_type + 対象 cell 型 + 許容 morphism_type 集合）まで自動適用してよいか」を宣言する。**plan の受理 = その範囲内の遷移適用の事前承認**。
- 実行時は決定論ゲートが守る: base revision 一致、morphism check、precondition/postcondition、invariant 再検査、evidence origin 検査（inferred は hard requirement を満たさない）。ゲートを通らない遷移は obstruction として記録され、unreviewed のまま残る（自動リジェクトもしない）。
- Worker 出力・LLM 提案はすべて untrusted で受領し、`source_backed` 証拠（content_hash 付き）と unreviewed 遷移候補として記録する。plan が事前承認しない遷移クラスは人間 review 待ちになる。

**D7. Worker 境界は adapter 層に隔離**。lib（モデル・評価・store）は純粋を維持し、`std::process` 等の副作用は `exec::worker` アダプタモジュール（CLI から呼ばれる層）のみに置く。`semantic_proof_backend.rs` の信頼境界パターン（出力ハッシュ化 + untrusted マーク）を踏襲する。legacy の教訓に従い、**実効果 worker の有効化はセキュリティ/承認ポリシーのパスを別フェーズに置く**（最初の worker は「宣言済み作業ディレクトリ内・環境変数 allowlist・タイムアウト付きの単発プロセス」に限定）。

**D8. 最初の実行サーフェスは `run --step` のみ**。1回の呼び出しで frontier から1件だけ進める。常駐 daemon・分散 scheduler・retry 基盤・event bus・並列 dispatch は導入しない。選択ポリシーは「accepted plan に列挙された順で、frontier に含まれる最初の項目」という決定論とし、優先度エンジンは作らない。

**D9. バージョニング**: 新レコードはすべて新スキーマID（`highergraphen.case.workflow.execution_plan.v1` 等）とし、既存 v1 スキーマへのフィールド追加はしない（strict v1 / unknown fields rejected の規約に従う）。レポートの `tool_package` 値は切り出し後の実体に合わせて `"casegraphen"` とする。

---

## 3. 目標アーキテクチャ

### 3.1 P / E / R の実現

| 構造 | 実現 |
|---|---|
| **P**（規範的プロセス） | native case space の accepted 部分: goal/work/decision セル、hard relation、close policy、EvidenceRequirement、Capability/Policy セル（`custom:governed_by_policy` / `custom:allowed_by_capability` relation） |
| **E**（処理グラフ） | **ExecutionPlan**（新規レコード）: 固定 revision + WorkItem→WorkerBinding の列 + 各項目の入力射影プロファイル + 成功条件（EvidenceRequirement 参照）+ 自動適用を許す遷移クラス。plan 生成は P→E の **Lift Morphism** であり、保存されたもの（goal/順序制約/権限/完了条件/不変条件の対応 ID）と失われたもの（自動化できなかった構造・曖昧さ）を `information_loss` 語彙で必須記録する |
| **R**（実行記録） | morphism log entry + **ExecutionTrace**（新規レコード: dispatch した項目、worker、入力射影、出力ハッシュ、出現/解消 obstruction、attach した evidence）+ `EvidenceRecord`（`source_backed`, `content_hash`, `captured_by`） |
| **R→P/E**（Evidence Morphism） | 「コマンドが成功した」= `EvidenceType::CommandOutput` の記録。「プロセス上の目的が達成された」= その証拠が EvidenceRequirement を満たし（origin 検査込み）、成功条件・不変条件の再検査が通り、`evidence_attach` + `update` morphism が受理されること。この2段を混同しない |

### 3.2 実行ループ（`run --step`）と既存 API の対応

```
1. plan 読込 + revision 固定     ExecutionPlan(accepted) / replay_case_space + checksum 照合
2. readiness 再導出              evaluate_readiness（保存値を信用しない。再導出のみが合法）
3. WorkItem 1件選択              frontier_cell_ids ∩ plan、plan 順で先頭
4. ゲート検査                    NativeOperationGate を operation=dispatch に拡張
                                 + CapabilityOperation::ExecuteMorphism
                                 + plan/item の ReviewStatus 検査
5. 入力射影を Worker へ           Projection(audience=system) + InformationLoss 必須
                                 + measure_projection_loss で実損失を記録
6. WorkerReport 受領             新規 wire 契約（3.3）。全出力 untrusted
7. 遷移を morphism として検査     check_morphism + precondition/postcondition
                                 + typed reducer による適用可能性検査
8. 成功条件・不変条件再検査       EvaluatorKernel + CheckInput::changed_cells（増分）
                                 + evidence origin ゲート
9. 新 revision として記録         append_morphism（sequence/stale/checksum 検査、
                                 previous_entry_hash を今回から populate）
10. 履歴・損失の保存              ExecutionTrace + projection loss + obstruction 差分
```

各工程の失敗は **domain finding（obstruction）であり tool failure ではない**（stale base と checksum 不一致のみ tool failure）。ループはどの工程で止まっても morphism log から replay 可能。

### 3.3 新規 wire 契約（すべて新スキーマ ID・strict）

- `highergraphen.case.workflow.execution_plan.v1` — ExecutionPlan。`plan_id`, `case_space_id`, `base_revision_id`, `steps[]`（`work_item_id`, `worker_binding_id`, `input_projection_profile_id`, `success_evidence_requirement_ids`, `allowed_transition_classes[]`）, `provenance`, `review_status`。
- `highergraphen.case.workflow.worker_binding.v1` — WorkerBinding。`worker_kind`（初期は `shell` のみ）, `command`, `args`, `working_directory`, `env_allowlist`, `timeout_ms`, `capability_ids`。legacy の Worker protocol データ契約を出発点にする。
- `highergraphen.case.workflow.worker_report.v1` — Worker 出力。`outputs[]`（content_hash 付き）, `observed_side_effects[]`, `evidence_items[]`, `proposed_transitions[]`（CaseMorphism 候補, 必ず unreviewed）, `exit_status`, `trust_boundary`（`semantic_proof_backend` の語彙を踏襲）。
- `highergraphen.case.workflow.execution_trace.v1` — ExecutionTrace（3.1 参照）。

### 3.4 モジュール構成（本リポジトリ）

```
casegraphen/
  Cargo.toml                 # 単一パッケージで開始（workspace 分割は実行層が安定してから判断）
  src/                       # tools/casegraphen を移設（legacy 世代は削除）
    exec/                    # 新設: plan, gate, loop, trace（純粋ロジック）
    exec/worker/             # 新設: Worker trait + shell adapter（std::process はここのみ）
  schemas/                   # schemas/casegraphen/** を移設 + 新規4契約
  tests/fixtures/            # 統合テストが要する参照 fixture を移設
  docs/design/  docs/adr/
```

---

## 4. 移行計画

### Phase 0 — 境界改定（HigherGraphen 側）
1. ADR: casegraphen の切り出しと実行制御追加を決定として記録。`casegraphen.md` の非ゴール2条項（外部実行・外部リポジトリ不変更）の改定を明記。
2. crates.io の `casegraphen` crate の publication 主体を本リポジトリへ移す方針を記録（同一 org のため機械的。`repository` フィールド更新、バージョンは 0.8.0 から独立採番）。

### Phase 1 — 機械的切り出し（機能変更なし、テスト green が完了条件）
1. `tools/casegraphen/src`・`tests`・`schemas/casegraphen/**`・参照 fixture を本リポジトリへ移設。
2. Cargo.toml の workspace 継承をインライン化し、`higher-graphen-*` を公開バージョンに固定。`higher-graphen-projection` 依存を削除。
3. legacy 世代（`model.rs`/`eval.rs`/`report.rs`/`LocalCaseStore` と対応 CLI 分岐）、`native_report.rs` を削除。
4. `include_str!` 相対パス（約14箇所）と統合テストの `repo_path()` を新レイアウトに合わせて修正。CI に `python3 -m jsonschema` を用意（Rust 化は別判断）。
5. HigherGraphen 側: workspace members から除去、`examples/architecture` の path 依存を公開 crate 参照へ変更（または該当テストを移設）、`scripts/check-static-limits.py`・`validate-json-contracts.py` から casegraphen を除去。

### Phase 2 — 実行の前提基盤（HigherGraphen 自身の Step 8 計画と同内容）
1. **typed reducers**: `CaseMorphism` の `added/updated/retired_ids` + payload から cell/relation を実体化する reducer を `apply_bounded_morphism` に実装（最重要ギャップ）。
2. `review accept|reject|reopen|waive` コマンドの実装（現状はメタデータ morphism の手動 append のみ）。
3. 証拠の書き込みパス: `evidence attach` コマンド（`evidence_attach` morphism + EvidenceRecord 実体化）。
4. `previous_entry_hash` の populate と検証を有効化。
5. `WorkItemState` / `CaseCellLifecycle` の合法遷移表を定め、`state_transition` を初めて構築する writer を追加。

### Phase 3 — 計画と権限
1. ExecutionPlan スキーマ + `plan propose|check|accept|reject` コマンド（plan は completion candidate と同じ review 規律に乗せる）。
2. `NativeOperationGate` の operation に `dispatch` を追加し、per-operation gate 表の未配線行（Propose/Apply a morphism）を実装。

### Phase 4 — Worker binding と `run --step`
1. WorkerBinding / WorkerReport スキーマ + `Worker` trait + shell adapter（作業ディレクトリ限定・env allowlist・timeout・出力ハッシュ化）。
2. `run --step` コマンド: 3.2 のループを配線。ExecutionTrace 記録。
3. 統合テスト: 「plan 受理 → step 実行 → 証拠 attach → 遷移適用 → close-check」のエンドツーエンド 1 本を golden report 付きで。

### Phase 5 — セキュリティ/承認ポリシーのパス（実効果 worker の有効化条件）
legacy が保留した理由そのもの。実プロジェクトへの適用前に: worker の権限モデル（capability と OS 権限の対応）、approval policy（どの transition class が人間承認必須か）、監査射影（audit audience への ExecutionTrace 投影）をレビューする。これを通過するまで shell adapter はデフォルト無効（explicit opt-in）とする。

---

## 5. リスクと未決事項

1. **依頼文書の 5.2「実行バインディング」節が途中で切れている**。Worker binding の要求詳細（対象 worker 種別、リモート実行の要否など）が追加である可能性が高い。本設計は §5.1 の最小ループ + legacy の Worker protocol から導いた。追加要求があれば 3.3 の WorkerBinding 契約を拡張する。
2. **`EvaluatorCheck` が閉じた enum** のため、ドメイン固有の成功条件述語を crate 側に足せない。当面は tool 側で EvaluatorKernel の外に成功条件検査を実装し、必要になった時点で HigherGraphen 側へ「カスタム述語の登録機構」を提案する（他の型には `Custom` 拡張慣行が既にある）。
3. **`check_preservation` は宣言照合**であり、preservation の実証には遷移前後で `EvaluatorKernel` を回す必要がある。ループ工程8はこれを前提に設計した（工程7の check だけで信用しない）。
4. **スキーマIDのRust定数手動複製**によるドリフト。移設時は現状維持とし、契約バージョン更新時に単一ソース化（もしくは Rust jsonschema 検証の導入）を再検討する。
5. **crates.io の名前とバージョン連続性**: 既公開 `casegraphen` 0.7.x は HigherGraphen リポジトリ産。0.8.0 を本リポジトリから出す際、READMEとリリースノートで系譜（legacy → HigherGraphen 内蔵 → 独立）を明記する。
6. `report-schema-aliases.json` の ECMAScript 否定先読みは Rust `regex` で扱えない。Rust 側でエイリアス解決を実装する場合は `fancy-regex` か手動分岐が必要（現状はテスト/文書のみの消費なので影響なし）。
