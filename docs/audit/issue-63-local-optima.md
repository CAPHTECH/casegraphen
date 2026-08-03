# Issue #63 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: Graph Engineering Plane の公開 product surface、operational MCP host、control-plane catalog、CLI/MCP parity、README/Skills/usage/package conformance。
- 主要な結論: 各 workflow に個別 CLI 実装を作らず canonical Rust decision owner へ委譲する host は、調査した境界では `harmless-locality` である。一方、experimental v0 の全 tool payload を共通 `Value` に置く設計は、利用 runtime が増えるライフサイクル境界で優位性が反転する `time-delayed` 候補である。
- 高確度候補数: 1件。
- 証拠上の制約: 実運用 client の変更頻度、support 問い合わせ、長期 latency/SLO は未観測。静的構造、実行テスト、既存 ADR、package gate の証拠に限定した。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- standalone client が custom Rust host を書かず、同一の判定規則で topology から review seam まで到達できること。
- runtime output や redesign proposal を acceptance と誤認せず、hash、revision、gate、untrusted boundary がすべての入口で保持されること。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | 一つの MCP tool invocation | CLI、MCP、canonical module、Skills、package、operator を含む E2E |
| `M` 評価指標 | 少ない adapter code、tool が呼べること | 判定 parity、誤受理防止、client validation、変更増幅、運用 refusal |
| `N` 変更可能範囲 | operational host と catalog | host、wire schema、inventory、docs、Skills、quality gate |
| `T` 時間軸 | experimental v0 の今回リリース | 複数 runtime/client と stable promotion 後の反復変更 |

制約は Rust 1.80、既存 stable ledger の互換性、MCP stdio、scheduler/model/retry ownership の非移管である。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `docs/product-surface.v0.json:1-22`, `src/bin/casegraphen-mcp-host.rs:116-330` | 8 workflow と canonical delegate | host match の静的観測 |
| 実行 | `tests/product_surface.rs:19-167`, `tests/mcp_stdio.rs` | inventory gate、CLI/MCP lint parity、simulation parity、accept refusal、process E2E | production workload ではない |
| 進化 | `scripts/product-surface-conformance.py:11-66`, `scripts/static-analysis.sh` | catalog/ADR/README/Skills/package drift | 今回以前の共変更統計は未取得 |
| 意味・組織 | ADR 0019/0020、Issue #63 acceptance | ledger owner と runtime adapter owner の分離 | 実組織の担当時間は未観測 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | tool payload が共通 `serde_json::Value` | v0 語彙を速く変更でき、transport schema が小さい | client が tool 別 shape を discovery/schema から検証できず、host DTO と docs を追う | 複数 client / stable promotion | 7 | C2 | `time-delayed` |
| 2 | 一つの operational host match に全 workflow を集約 | decision owner を複製せず、認証・durability・refusal を一箇所で共有 | host file の fan-in と review 面積 | module | 4 | C2 | `harmless-locality` |

## 5. 上位候補の詳細

### Candidate PS-01: 共通 Value payload

#### 事実・推論・仮説

- [Evidence] `ControlPlaneRequest.payload` は `Value` であり、MCP `tools/list` も payload を `{}` として公開する。
- [Evidence] host は `deny_unknown_fields` の tool-specific DTO に直ちに変換し、未知/欠落 field を `invalid_payload` で拒否する。したがって server-side fail-closed は成立する。
- [Evidence] wire catalog、request enum、product inventory、ADR、README、Skills、Cargo/usage は `scripts/product-surface-conformance.py` で結合され、対象テストと Rust 1.80 clippy が通過した。
- [Inference] 現在の不利益は authority hole ではなく、client-side discovery と versioned tooling の弱さである。
- [Hypothesis] 複数言語 client と stable contract が増えると、事後 refusal と手書き DTO の更新負担が tool ごとの input schema 導入コストを上回る。

#### 局所的合理性

- 局所目的: experimental v0 の破壊的変更余地を残しながら全 workflow を一つの transport へ接続する。
- 局所指標: adapter 実装量、canonical module reuse、追加 tool のリードタイム。
- 直接の受益者: CaseGraphen maintainers と初期 host integrator。
- 現在も有効な利益: server-side strict DTO、typed refusal、durable idempotency、共通 response schema。
- 導入時/現在も有効な制約: v0 語彙は実 runtime pilot で変更し得る。
- 失効した制約: なし。stable promotion 後は再評価が必要。

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| payload schema を `{}` に保つ | client が呼出前に shape を検証できない | ADR/guide/Skill と server-side DTO refusal | client/operator | tool 導入・変更ごと | `mcp_stdio::tool_definition`, host DTO |
| tool/result schema を共通 envelope に置く | result の tool-specific discovery が弱い | inventory の canonical owner と result boundary を参照 | SDK author | client 追加ごと | `docs/product-surface.v0.json` |

#### 四観測面

- 構造: transport の `Value` と host 内 strict DTO の二段階境界がある。
- 実行: malformed/unsupported operation は typed refusal、lint/simulation は canonical report と完全一致する。
- 進化: conformance gate は文字列 drift を防ぐが、tool-specific JSON shape 自体は schema inventory にない。
- 意味・組織: runtime adapter は観測を提案へ変換できるが、acceptance ledger の review/mutation owner にはならない。

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 少ない generic protocol code | runtime parse | tool-specific validation | schema dispatch | 現在案 |
| モジュール | strict DTO で fail closed | DTO は host private | exported input types | public API 増加 | 現在案 |
| 機能 | 8 workflow を即時提供 | client 事前検証なし | per-tool input schema | inventory/schema追加 | 同等 |
| システム | canonical decisions と一つの durable boundary | discovery が粗い | schema-aware SDK | generator/tooling | 代替案候補 |
| 運用・組織 | refusal が一様 | operator が payload detail を読む | validation at client edge | schema release coordination | 代替案候補 |
| ライフサイクル | v0変更が容易 | stable client の変更増幅 | versioned per-tool schemas | 移行中二重 schema | 代替案 |

- 反転する最小境界: 複数の独立 client が stable contract に依存するシステム境界。
- 反転する指標: client-side failure detection、変更増幅、support burden。
- 反転する時間軸: experimental v0 から stable promotion を検討する時点。

#### 反実仮想

- A 現状維持: v0 pilot は速い。stable 後は docs/DTO 追従と事後 refusal が累積する。
- B 最小改善: inventory に tool ごとの example/input/output schema id を追加する。移行は容易だが MCP discovery は依然 generic。
- C 境界変更: `tools/list` が tool-specific versioned JSON Schema を返し、同じ schema を Rust conformance と SDK generator が使用する。移行中は v0 generic payload と typed payload の二重化が必要で、rollback は inventory flag で可能。

#### スコアと判定

- `E=2`, `A=2`, `F=1`, `K=0`, `T=2`, `Severity=7`, `Confidence=C2`。
- 分類: `time-delayed`。
- 反証: 実 runtime pilot 後も client が一つで、stable promotion を行わないなら局所利益は反転しない。

## 6. 横断的な補償構造

- 共通する変換: MCP `Value` → tool-specific strict DTO → canonical owner report。
- 共通する例外分岐: `invalid_payload` と `unsupported_operational_host_tool`。
- 再試行・手動運用: durable pending marker が曖昧な prior effect の自動再実行を避け、operator reconciliation を要求する。
- 所有権: inventory が entry point と canonical owner を固定し、host は scheduling/model/retry/acceptance ownership を持たない。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| 一つの host match | 8 workflow の fan-in | canonical decision は各 module に残り、lint/simulation parity が実測され、共通の認証・durability・refusal だけを集約 | transport adapter の意図的集約 |
| main CLI に全 command を追加しない | CLI surface が少ない | operational MCP host が package され、no-custom-Rust E2E と inventory/usage/install gate がある | stable ledger CLI と experimental runtime surface の分離 |
| expansion を一 invocation の bounded rounds にする | daemon state を保持しない | all-seen/dry/budget は一 controller 内で維持し、proposal が出た時点で review seam に停止する | host が scheduler/standing consent を所有しないための境界 |

## 8. 未検証事項

- 実 MCP client SDK が全8 workflow を継続運用した際の payload error rate。
- streaming resource permit を含む production-scale latency と restart behavior。
- stable promotion 時の tool-specific schema migration cost。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | 二つ以上の外部 runtime client transcript | generic payload の負担が反転したか | pilot ごとに refusal と修正回数を保存 |
| 2 | 全8 workflow の golden MCP transcripts | tool wrapper の E2E drift | release fixture と canonical report diff |
| 3 | operator retry/restart metrics | durable ambiguity refusal の運用負担 | host supervisor telemetry |

## 10. 介入判断の前提

- experimental v0 の間は現構造を維持し、stable promotion review で per-tool schema を判断する。
- 移行時は generic envelope の replay compatibility を一期間維持する。
- acceptance ledger、scheduler、runtime の所有境界は変更しない。
- rollback は product inventory と catalog を直前の experimental version へ戻せることを要件とする。
