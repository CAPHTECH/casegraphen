# Issue #85 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: remote transport、binary artifact、512-node runtime reconciliation、allocator journal、reviewed resource host、retained evidence。
- 主要な結論: pilot専用oracleで“成功”を作る局所最適を除去し、binary/scale completenessをproduction canonical reconcilerへ委譲した。reviewed deployment hash、raw inputs/outputs、thresholdsを一つのmanifestへ固定した。
- 高確度候補数: 修正済み2、未解消1（allocator checkpoint）。
- 証拠上の制約: loopback/local process、512 events、provider host attestationは#76。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- 実runtimeのtransport/artifact/scale/journal failureがgraph completenessやauthorityを偽陽性にしないこと。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` | pilot harness内 | canonical protocol、host、allocator、retention |
| `M` | lane終了 | edge proof、latency/RSS、authority binding、fail closed |
| `N` | Python harness | Rust reconciler/examples、host E2E、docs/report |
| `T` | normal run | timeout/disconnect/retry/restart/crash/long journal |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | runtime/resource modules | canonical ownerへの委譲 | local build |
| 実行 | retained issue-85 evidence | TCP、binary、512/511/128、512 journal | single host |
| 進化 | promotion review、#76/#85 | stable blocker | remote fleet未実施 |
| 意味・組織 | accepted:false、review seam | authority separation | operator review未実施 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | pilot専用edge oracle | 実装が速い | production ruleの性能/正しさを測れない | canonical reconciler | 12 | C3 | 修正済み |
| 2 | summaryだけ保持 | artifactが小さい | 再検証不能 | release evidence | 10 | C3 | 修正済み |
| 3 | journal全replay | 実装が単純 | event数比例のappend/restart cost | long-lived allocator | 8 | C3 | 未解消・明示 |

## 5. 上位候補の詳細

### Candidate I85-C1: pilot専用edge oracle

- [Evidence] `examples/runtime_reconciliation_durability_pilot.rs`は512-node topologyからcanonical expectationを導出し、640 typed reportと実bytes observationsを`reconcile_runtime_reports`へ渡す。
- [Evidence] retained completenessは511 edge proof、node/dataflow/completeを分離する。
- [Inference] pilotとproductionでterminal/retry/schema/parent/artifact ruleがdriftしない。
- [Hypothesis] さらに大きいfan-out graphではchainと異なるmemory profileになる。
- 局所目的/利益: Python集合演算なら短く高速に見える。
- 外部化: productionのretry lineage、typed edge、byte observationを一切測れない。
- `B/M/N/T`: harness→production boundary、件数→canonical completeness、Python→Rust owner、normal→retry。
- 補償ハロー: 手製tuple hashをedge proofと呼び、reviewerが実装差を手動確認する。
- 四観測面: Rust owner再利用、実report実行、contract hash retention、accepted:false。
- 反転する最小境界: stable-promotion evidence。
- 反実仮想A: 手製oracle維持、B: testだけcanonical、C: executableもcanonical（採用）。
- スコア: E3/A2/F3/K2/T2 = 12、C3。
- 判定: `metric_shift`修正済み。pilot速度ではなくproduction completenessを測る。

### Candidate I85-C2: summary-only retention

- [Evidence] manifestはtopology、expectation、640 reports、completeness、binary bytes、remote journal、allocator/reviewed-resource reportをhash/length付きで保持する。
- [Inference] summary値をraw evidenceから再計算できる。
- 局所目的/利益: small artifact。
- 外部化: failure investigationとindependent verification不能。
- 反転する最小境界:別reviewer/次release。
- 反実仮想: summaryのみ、代表sample、全bounded raw evidence（採用）。
- スコア: E2/A2/F2/K2/T2 = 10、C3。
- 判定: `time-delayed`修正済み。

### Candidate I85-C3: allocator full replay

- [Evidence] 512 event appendはthreshold内だが、reportはcheckpoint/compaction `implemented:false`を保持する。
- [Inference] 長期運用ではappendごとのreplayが優位性反転する。
- 局所目的/利益: append-only source of truthと単純recovery。
- 外部化: operation latency、startup time。
- 反転する最小境界: journalがSLO上限を超えるevent数。
- 反実仮想: 現状、cache、hash-bound checkpoint/compaction。今回は診断のみ。
- スコア: E2/A1/F2/K1/T2 = 8、C3。
- 判定: `time-delayed`未解消。stable blockerとして保持。

## 6. 横断的な補償構造

- 共通変換: runtime claim→手製summaryを廃止しcanonical typed resultを保持。
- 例外分岐: remote failureはidempotency journal、runtime failureはretry lineage。
- 再試行: timeout/disconnect/retryを成功件数へ隠さない。
- 所有権: #76はprovider provenance、#85はgraph/runtime durability。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| loopback TCP | production remoteでない | process/socket failure境界をboundedに再現 | local deterministic pilotとして合理的 |
| unreviewed allocator mechanics | authority不足 | 別reviewed-resource laneがexact bundle bindingを実証し、mechanics laneはcrash/scaleへ限定 | 責務分離として合理的 |

## 8. 未検証事項

- remote scheduler、binary >64KiB、fan-out scale、allocator checkpoint、provider-host attestation。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | broker-signed provider runs | host/session provenance | #76 runner |
| 2 | 10k/100k journal | replay反転点 | checkpoint design前後benchmark |
| 3 | large fan-out/fan-in | chain以外のmemory | canonical topology matrix |

## 10. 介入判断の前提

- 変更範囲: experimental runtime/resource/pilot。
- 移行期間: v0中。
- 一時悪化可能: evidence artifact size。
- 制約: pilot failureはaccepted stateを変えない。
- ロールバック: retained raw evidenceとsource hashから旧/new harnessを比較可能。
