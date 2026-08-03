# Issue #59 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査モード: deep-dive / intervention
- 調査範囲: streaming acceptance、early-release resource permit、runtime resource reconciliation provenance
- 主要な結論: revision検査だけを`reconcile_stream`へ足す局所修正では、公開フィールドを持つintegration reportのresource reconciliation差し替えが残る。permitをrevision・target・attempt・reconciliation bytesへ結合し、生成元側の非wire provenanceも同じhashへ結合した。
- 高確度候補数: 1（実装中に解消）
- 証拠上の制約: 実runtimeのlatency・障害履歴は未取得。判定は静的構造、実行テスト、Git履歴に基づく。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- revision Aのreadinessやresource reconciliationをrevision Bのearly releaseへ再利用できないこと。
- replay順序に依存せず、同じ入力から同じproposal/findingを得ること。

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | `reconcile_stream` | canonical case evaluation → runtime integration → streaming release |
| `M` 評価指標 | release可否 | provenance差し替え耐性、replay決定性、変更時の不変条件維持 |
| `N` 変更可能範囲 | streaming module | runtime integrationの非wire provenanceと統合テストを同時変更 |
| `T` 時間軸 | 1回のreconcile | case revision更新、retry/replay、将来のhost integration |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/streaming_reconciliation.rs:88-242,297-330,493-541` | opaque permit生成・照合 | 静的証拠 |
| 実行 | `cargo test --test streaming_reconciliation` | 10 tests、stale/replay/substitutionを含む | reference fixturesのみ |
| 進化 | `git_local_optima_signals.py --since "12 months ago"` | 107 commits、中央値3 files/commit | issue固有の共変更回数は少ない |
| 意味・組織 | Issue #59、`src/runtime_integration.rs:88-128,394-447` | acceptance ownerとruntime observation ownerの境界 | team/SLO情報なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | streaming側だけでpermitをhash化 | 変更が一moduleに閉じる | mutable reportのreconciliation bytesと生成provenanceが乖離 | runtime integration境界 | 9 | C2 | mixed（解消済み） |

## 5. 上位候補の詳細

### 5.1 識別

- Candidate ID: I59-C1
- 名称: streaming-local resource binding
- 対象実装: `derive_streaming_resource_permits`
- 所有モジュール: `streaming_reconciliation` / `runtime_integration`

### 5.2 事実・推論・仮説

#### 観測された事実

- [Evidence] `RuntimeIntegrationReport.resource_reconciliations`は公開される一方、canonical provenanceは`serde(skip)`のprivate setである（`src/runtime_integration.rs:101-108`）。
- [Evidence] permit生成はreconciliationのcanonical JSON hashを再計算し、生成時に保存された同一hashを要求する（`src/streaming_reconciliation.rs:182-221`）。
- [Evidence] testは`actual_allocation_count`だけを変更したcloneがpermitを得られないことを検証する（`tests/streaming_reconciliation.rs`の`resource_permits_refuse_cross_graph_and_cross_node_substitution`）。

#### 推論

- [Inference] streaming側で現在見えているbytesだけをhash化しても、callerがreportとhash対象を一緒に差し替えられる。生成元の非wire provenanceとのjoinが必要である。

#### 未検証仮説

- [Hypothesis] 大規模runtimeでpermit mapのhash保持コストが支配的になる可能性は低いが、実測していない。

### 5.3 局所的合理性

- 局所目的: streaming moduleのAPIだけでrevision bindingを完結する。
- 局所指標: 変更ファイル数と実装量。
- 直接の受益者: streaming adapter実装者。
- 現在得られる利益: 単純な依存方向。
- 現在も有効な制約: runtime outputをauthorityにしない。
- 失効した制約: runtime integrationを変更できないという制約はない。

### 5.4 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| streamingだけでhashを作る | integration reportの生成時bytesを証明できない | caller規律または追加検査 | host integrator | permit生成ごと | substitution testが局所案を反証 |

### 5.5 四観測面の証拠

- 構造: opaque permitはtopology/revision/node/attempt/resource hashをprivate保持する。
- 実行: stale revision、empty revision、graph/node/resource substitution、duplicate/out-of-order replayが通過。
- 進化: hash join追加は2 module + integration testに限定され、履歴上の大規模hotspotを増やしていない。
- 意味・組織: readinessはcanonical evaluator、resource observationはruntime integratorが所有し、streaming reconcilerはjoinだけを行う。

### 5.6 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 単純なhash比較 | なし | provenance joinは引数増 | 少量 | 局所案 |
| モジュール | streaming内に閉じる | report差し替えを検出不能 | integration生成時hashとjoin | module間変更 | 代替案 |
| 機能 | revisionだけは保護 | resource result replayが残る | release tuple全体を保護 | fixture更新 | 代替案 |
| システム | host実装が補償 | caller依存 | fail closed | なし | 代替案 |
| 運用・組織 | 実装担当が局所完結 | integratorが暗黙責任を負う | owner境界が型に反映 | coordination | 代替案 |
| ライフサイクル | 初期変更が小さい | 新adapterごとに再発 | provenance invariantを再利用 | v0 API変更 | 代替案 |

- 反転する最小境界: module間のruntime integration境界
- 反転する指標: 実装量からprovenance substitution耐性へ広げた時
- 反転する時間軸: 2回目のrevision/replayから

### 5.7 反実仮想

#### A. 現状維持

- 定常コスト: stale readinessとcoarse permitの再利用余地。
- リスク: 未reviewのearly-release proposalが誤ったrevision/attemptへ結合する。

#### B. 最小限の局所改善

- 変更: `expected_case_revision_id`比較だけを追加。
- 利益: acceptance A/B mismatchは拒否。
- 残る問題: resource reconciliation bytesの差し替え、node/attempt/hashのproposal上の不可視性。

#### C. 境界をまたぐ構造変更（採用）

- 変更: canonical evaluator由来revisionとruntime integrator生成時reconciliation hashをopaque permitへ結合。
- 成立条件: resource reconciliationが決定論的にserialize可能。
- 定常利益: tupleの一要素だけを差し替えたpermit生成をfail closed。
- 新たなコスト: runtime integrationとstreamingの意図的な共変更。
- 移行の谷: experimental v0 API callerがacceptance引数とexpected revisionを追加する必要。
- ロールバック: v0のため可能だが安全性を失う。

### 5.8 スコアと判定

- `E`: 2
- `A`: 1
- `F`: 3
- `K`: 1
- `T`: 2
- `Severity`: 9/15
- `Confidence`: C2
- 分類: `mixed`（externalization + time-delayed）、実装中に解消
- 反証となり得る情報: integration reportが外部から一切変更不能である証明。ただし現型はpublic fieldsと`Clone`を持つ。

## 6. 横断的な補償構造

- 変換: canonical JSON hashをintegration生成時とpermit生成時に照合する。
- 例外分岐: stale/empty/mismatchはtyped findingへ統一。
- 再試行・手動運用: duplicate/out-of-orderはlogical sortとidentity dedupeで決定論的。
- 所有権: evaluator、runtime integrator、stream reconcilerのdecision ownerを混ぜない。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| resource permitにcanonical acceptanceを必須化 | resource-only targetまでcase revisionへ結合 | issue要件がevery permitのrevision bindingを要求し、replay境界を一様にする | intentional safety coupling |
| terminal-only reconcileにもexpected revisionを要求 | releaseしないrunにも引数追加 | API全体で観測revisionを明示し、callerによる暗黙current置換を防ぐ | experimental v0で妥当 |

## 8. 未検証事項

- 実runtimeでのpermit derivation throughput、長時間streamでのmemory profile。
- 外部host adapterのAPI移行（Issue #63/#69の範囲）。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | 実runtimeのrevision advance/retry trace | fixture外でstale refusalが発火するか | runtime integration pilot |
| 2 | permit derivation benchmark | hash計算の運用コスト | 1k/10k reconciliation benchmark |

## 10. 介入判断の前提

- 変更可能範囲: experimental streaming/runtime integration API。
- 許容移行期間: v0中の破壊的変更として即時。
- 一時的悪化: callerの引数追加、hash計算1回。
- 互換性制約: stable ledger semanticsは変更しない。
- ロールバック要件: fail-openへ戻さず、旧callerはcompile failureで検出する。
