# Issue #86 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: stream event、release proposal、MCP product surface、Skill、ADR、simulation boundary。
- 主要な結論: 全graph barrierより早いという局所的利益を“streaming”と総称し、chunk-level overlapまで実装済みに見せる意味の外部化を解消した。v0は`terminal_artifact_stage_pipelining_v0`と機械可読に固定した。
- 高確度候補数: 修正済み1、未解消0。
- 証拠上の制約: true incremental streamingは未実装・未pilot。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- runtime利用者がrelease timing、failure/retry、resource lifetimeを誤認せず、安全なpipelineを設計できること。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` | graph全体barrierとの比較 | producer/consumer edge lifecycle |
| `M` | downstream開始が早い | overlap開始点と再実行安全性 |
| `N` | reconciler | schema、product surface、Skill、ADR |
| `T` | terminal event後 | producer実行中、crash、retry、resume |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/streaming_reconciliation.rs` | release predicate/result | v0 |
| 実行 | `tests/streaming_reconciliation.rs` | terminal report/final bytes必須 | local fixtures |
| 進化 | ADR 0024 | v0→incremental境界 | future contract未設計 |
| 意味・組織 | README、Skill、product inventory | consumer-facing vocabulary | external docs未観測 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | terminal stage releaseをstreamingと総称 | 短い既存名 | chunk overlap、backpressure、retry保証の誤認 | runtime integrator | 9 | C3 | 修正済み |

## 5. 上位候補の詳細

### 5.1 識別

- Candidate ID: I86-C1
- 名称: Ambiguous streaming product claim
- 対象実装: streaming reconciliation v0
- 所有モジュール / サービス: runtime integration
- 所有チーム: CaseGraphen
- 導入時期: early-release proposal導入時
- 調査者: Codex

### 5.2 事実・推論・仮説

- [Evidence] release predicateは`final_chunk`、canonical terminal attempt、terminal report output、byte observationをすべて要求する。
- [Evidence] resultとproposalは`release_semantics: terminal_artifact_stage_pipelining_v0`を返す。
- [Evidence] product inventory、event/simulation schema、README、Skill、ADRはchunk-level overlapではないと明記する。simulation requestも旧`streaming_overlap_basis_points`を廃止し、固定されたrelease semanticsを要求する。
- [Evidence] public resultは`stage_release_proposals`／`stage_release_blocked`を使い、旧`early_release`語彙を残さない。simulation reportは選択されたrelease semanticsと、terminal artifact契約上のdirect producer/consumer overlapが常に0msであることを型付きrangeとして返す。独立stage間のevent-driven overlapは通常のDAG latency simulationへ含まれる。
- [Inference] v0はslow siblingを待たないがproducer実行中にconsumerを開始しない。
- [Hypothesis] consumer側はtool名`reconcile_streaming_run`だけを見て従来の意味を推測する可能性がある。

### 5.3 局所的合理性

- 局所目的: frontier barrier前に次stage proposalを出す。
- 局所指標: graph全体完了より前のrelease。
- 直接の受益者: external runtime adapter。
- 現在得られている利益: stage latency短縮。
- 導入時の制約: incremental byte/consumer state contractがなかった。
- 現在も有効な制約: runtime outputはuntrusted、retry lineageはcanonical terminalで決定。
- 失効した制約: semanticsをwire resultへ露出できない制約はない。

### 5.4 評価条件

- `B`: reconcilerからruntime scheduler/user expectationsへ拡張。
- `M`: release有無からproducer/consumer overlap時点へ拡張。
- `N`: naming、schema description、result、docsを変更可能。
- `T`: normal completionからpartial retry/crashへ拡張。

### 5.5 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| streamingという総称 | adapterがchunk authorityを推測 | 実装読解・個別説明 | integrator | adapterごと | review #86 |

### 5.6 四観測面の証拠

- 構造: terminal completeness projectionをreleaseが再利用する。
- 実行: final eventだけでは足りずterminal reportとobserved bytesが必要。
- 進化: incremental streamingは別versionを要求するADRに固定し、simulationのunknown metricもterminal stage release timingへ限定した。
- 意味・組織: compatibility tool名とnormative semanticsを分離。

### 5.7 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 明示案の利益 | 明示案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 短い型名 | predicate読解必要 | enumで自己記述 | field追加 | 明示案 |
| モジュール | 既存API | 意味が暗黙 | resultで固定 | serialization差分 | 明示案 |
| 機能 | pipelineを訴求 | overlap誤認 | 正確な能力表示 | 長い名称 | 明示案 |
| システム | runtime自由 | retry/backpressure期待ずれ | contract境界明確 | future v1必要 | 明示案 |
| 運用・組織 | 説明が短い | incident時責任不明 | operator期待一致 | migration説明 | 明示案 |
| ライフサイクル | 名前維持 | v1と衝突 | compatibility alias化 | version管理 | 明示案 |

- 反転する最小境界: runtime adapter。
- 反転する指標: “全graphより早い”から“producer実行中に重なる”へ変えた時。
- 反転する時間軸: partial consumption後にproducer retryが起きた時。

### 5.8 反実仮想

#### A. 現状維持

- 定常コスト: integratorごとの実装読解。
- 将来コスト: incremental版との名称衝突。
- リスク: unsupported chunk consumption。

#### B. 最小限の局所改善

- 変更: docsだけに注記。
- 利益: 人間には説明可能。
- 残る問題: machine consumerは識別不能。
- 移行コスト: 小。

#### C. 境界をまたぐ構造変更

- 変更: result/proposal/product inventoryにnormative enumを追加しdocsを統一（採用）。
- 成立条件: compatibility tool名を残す。
- 定常利益: machine/human双方で能力を過大評価しない。
- 新たなコスト: result field追加。
- 移行の谷: strict response consumerの更新。
- ロールバック: field削除可能だが意味の曖昧さが復活。

### 5.9 スコア

- `E` 2、`A` 2、`F` 2、`K` 2、`T` 1。
- `Severity`: 9/15。
- `Confidence`: C3。

### 5.10 判定

- 分類: `metric_shift`（修正済み）。
- 判定理由: graph barrierとの比較指標では有利だが、edge overlap指標へ広げると“streaming” claimが反転した。
- 反証となり得る情報: producer terminal前releaseを示す実trace。
- 未検証事項: strict MCP response consumer compatibility。
- 次に取得すべき証拠: independent clientでsemantics fieldをassert。

## 6. 横断的な補償構造

- 共通する変換: streaming→stage pipeliningの口頭変換をmachine fieldへ移した。
- 共通する例外分岐: final/terminal/bytes条件はcanonical predicateへ集約済み。
- 共通する再試行・手動運用: incremental retryは未提供と明示。
- 所有権・KPI: latency訴求だけでauthority/completenessを省略しない。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| `artifact_chunk` event | chunk streamingに見える | ordered observationとして将来にも利用でき、release authorityと分離済み | event vocabularyとして合理的 |
| compatibility tool名維持 | 曖昧 | response semantics、inventory、docsで限定し既存clientを壊さない | v0移行として合理的 |

## 8. 未検証事項

- strict response decoder、incremental v1、backpressure/crash-resume。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | independent MCP response | fieldの運用可視性 | client pilot assertion |
| 2 | incremental design spike | v1必要契約 | chunk/retry/backpressure simulation |

## 10. 介入判断の前提

- 変更範囲: experimental contractとconsumer docs。
- 移行期間: v0中にfield追加。
- 一時悪化可能: response sizeとclient更新。
- 制約: proposalはacceptance/dispatch authorityを持たない。
- ロールバック: compatibility nameは維持する。
