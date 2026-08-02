# Issue #45 実装局所最適監査

## 1. エグゼクティブサマリー

- 調査範囲: `ExecutionTopology` の静的解析、`graph lint` CLI、report schema、受入テスト
- 主要な結論: typed topologyを唯一の構造入力にする境界と、決定論的事実/heuristicの区別は維持されている。監査中に、worktree分離だけで同一file writerの統合まで安全と誤解され得る局所判断を発見し、直接衝突とは区別した `isolated_worktree_merge_risk` を出す形で外部化を可視化・再検証した。
- 高確度候補数: 修正済み1件。未修正の中重大候補は0件、弱い候補は2件。
- 証拠上の制約: 新規未commit実装のためGit進化履歴、運用trace、実fleetのcost/latencyはない。性能・組織コストの主張は限定する。

## 2. システム成果と評価条件

最終成果は、CaseGraphenの受理規則を複製せず、安全な並列性、不要edge、barrier、fan-in、policyの観測限界を再現可能に診断することである。

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` | 一つのlint関数とCLI invocation | design Skill、runtime統合、1000 nodeのfleet |
| `M` | 決定性、実装の単純さ、fixture通過 | signal/noise、schema安定性、統合時resource安全性、解析量 |
| `N` | lint module内 | topology contract、CLI、schema、consumer Skillを同時変更可能 |
| `T` | v0の一回のlint | policy contractが育つまでのexperimental期間と反復運用 |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/graph_lint.rs`, `src/execution_topology.rs`, report schema | graph構築、分類、resource比較、serialization | 静的証拠 |
| 実行 | `cargo test --test graph_lint`; CLIでshipped exampleをlint | 20-way、1000 fan-in、cycle、resource、schema/text | production workloadではない |
| 進化 | shared worktree diff | 今回の実装のみ | commit/PR履歴なし |
| 意味・組織 | ADR境界とIssue受入条件 | core decision ruleをlintへ複製しない境界 | 実運用者への聞き取りなし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | isolated worktreeをfile conflictの免除とする | dispatch時の直接write collisionを正しく避ける | merge/integration conflictをconsumerへ先送り | runtimeからend-to-end integration | 9 | C2 | `externalization`、修正済み |
| 2 | policy観測不能findingをnodeごとに出す | node位置が精密 | fleet/Skillで同一既知限界が大量反復 | 単一graphからfleet運用 | 6 | C2 | `time-delayed`候補、保留 |
| 3 | edgeごとの代替path探索と全node pairのresource比較 | 単純で監査可能な実装 | dense/large graphで解析量が増幅し得る | 1000 node fixtureから大規模dense graph | 5 | C1 | `insufficient-evidence` |

## 5. 上位候補の詳細

### Candidate LO-45-1: worktree安全性の評価境界

#### 事実・推論・仮説

- [Evidence] `resource_conflicts` はunorderedな同一resource writerを検出する。isolated worktreeは直接workspaceを分離するが、同じfileを変更した成果の統合可能性を保証しない。
- [Evidence] `tests/graph_lint.rs` のshared exclusive writer fixtureは決定論的errorを要求する。isolated file writerには別のheuristicを出し、修正後に全6テストが成功した。
- [Inference] dispatch単体ではworktree免除が合理的だが、成果統合まで境界を広げるとresource edgeなしの安全判定は反転する。
- [Hypothesis] 実運用でmerge conflictがどの頻度になるかは未計測。

#### 局所的合理性

- 局所目的: 異なるworktreeで同時編集できる並列幅を失わない。
- 直接の受益者: code-agent runtimeとscheduler。
- 現在も有効な利益: 直接のfilesystem write raceは分離できる。
- 局所変更だけでは改善しにくい理由: integration node/resourceの契約がv0にはない。

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| isolated worktreeなら同一file writerを安全とする | merge時に競合 | runtimeでの手動merge/retry | integrator/operator | writer fan-outに比例 | fixtureとresource semantics |

#### 優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数/dispatch | false positiveを減らす | mergeは見ない | conflictを残す | conservative | worktree免除 |
| 機能/runtime | 高いfan-out | integration failureを先送り | merge riskを明示 | review増加 | conflictを残す案 |
| ライフサイクル | 初期reportが静か | retry/手動解消が反復 | topologyで再設計可能 | edge設計コスト | conflictを残す案 |

#### 反実仮想

- A 現状維持: dispatchは速いがintegration riskがreport外になる。
- B 最小改善（採用）: isolated worktreeは直接write raceを回避するため決定論的conflictから除外する一方、`isolated_worktree_merge_risk`を必ず示す。shared writerは決定論的conflictのままにする。移行はreport finding追加のみでrollback容易。
- C 構造変更: integration node/resourceをv1 contractへ追加する。定常精度は高いがv0語彙の早期固定になるため今回は行わない。

#### スコアと判定

- `E=2, A=1, F=2, K=2, T=2`, Severity `9`, Confidence `C2`
- 分類: `externalization`
- 判定: dispatch boundaryの利益がintegration boundaryで反転する。最小改善を適用し、テストで再検証済み。

### Candidate LO-45-2: node単位の既知限界notification

- [Evidence] shipped 4-node exampleのCLI実測は5 findingsを出し、そのうち3件が同じ `verification_independence_uninspectable` だった。
- 局所利益: 各nodeのJSON locationが明確。
- 外部化コスト: policy単位で同じ限界を知りたいconsumerがdedupeする。
- 反転境界: 数nodeでは現在案、数百nodeを継続監査するfleetではpolicy単位の集約が優位になり得る。
- 反実仮想: A node単位を維持、B policy ID単位で集約、C reportにcapability/coverage sectionを追加。Bはtarget追跡情報を失うため、利用データなしに変更しない。
- `E=1, A=1, F=0, K=2, T=2`, Severity `6`, Confidence `C2`, 判定 `time-delayed`候補。

### Candidate LO-45-3: 単純な全探索

- [Evidence] redundant edgeはedgeごとにreachabilityを再探索し、resource conflictはnode pairを比較する。
- 局所利益: index/cache invalidationがなく決定的でレビューしやすい。1000 fan-in fixtureは単独テスト実測約0.20秒で成功した。
- 未検証仮説: dense graphまたは多数resource claimでは解析時間が支配的になる。
- `E=0, A=1, F=0, K=1, T=3`, Severity `5`, Confidence `C1`, 判定 `insufficient-evidence`。実測閾値なしにbitset/transitive closureへ置換しない。

## 6. 横断的な補償構造

- typed topologyから一つのadjacency/reverse表現を作り、case readiness・evidence・review・authorization判断は再実装していない。
- heuristicをdeterministic violationとして扱わない分類が、観測不能metadataを過剰に受理する補償分岐の増殖を抑えている。
- 現時点でretry、手動運用、チーム間調整の実証データはない。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| topology schemaとreport schemaの分離 | schema重複に見える | 入力契約と診断出力は異なる意味・進化境界 | 意図的なbounded context |
| deterministic/heuristicの二分類 | 分岐増加 | false-edge、authority、anchorはv0から証明不能 | 過剰なtruth claimを防止 |
| critical pathが全duration既知時だけ値を持つ | 部分値を捨てる | 不明durationを0とみなす方が誤解を招く | fail-closedなmetric |

## 8. 未検証事項と次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | 200/1000/5000 node、sparse/dense、claims有無のbenchmark | LO-45-3の反転点 | criterionまたはrelease fixture timing |
| 2 | consumerがfindingをどう集約するか | LO-45-2のnoise実害 | design/audit Skill evalのtrace |
| 3 | worktree成果のmerge成功率 | LO-45-1の実害規模 | reconcilerのintegration report |

## 9. 介入判断の前提

- report v0はexperimentalであるが、schema変更時はconsumer fixtureも同時更新する。
- 性能最適化はdeterministic orderingとfinding identityを維持する。
- verification/anchor/authorityの精度向上はpolicy contractを導入してから行い、CaseGraphen coreのdecision ruleをlintへ複製しない。
- report schemaのrollbackは可能にし、accepted case graphを自動変更しない。

## 10. 独立レビュー後の補正

初回監査後、異なるkindのparallel edgeを単なる到達可能性だけで「冗長」と判定する局所最適を検出した。探索関数だけを見ればkind非依存が簡潔だが、execution topology境界ではdata/authority/resource edgeは同じ端点でも別の契約を保護する（E3/A1/F2/K2/T1 = 9、C2、`externalization`）。A: kind非依存の到達判定、B: heuristicだけへ格下げ、C: 同じedge kindの代替経路だけをdeterministic redundancyとする、を比較してCを採用した。resource conflictの順序判定は全kindの到達性を引き続き使い、意味契約と実行順序の評価境界を分離した。

またLO-45-2は実装中にpolicy単位集約へ変更済みである。node単位targetを失う代わりにaffected node数をdetailへ保持し、fleet consumerへのdedupe外部化を除去した。

横断監査で#51の実policyが追加された後もlintが常に「uninspectable」とだけ報告する時間遅延型局所最適を検出した（E2/A2/F2/K2/T2=10、C3）。policy-aware entry pointを追加し、実validatorへ委譲してmissing/identity/shapeをdeterministicに、actor correlation・missing anchor・runtime attestation限界をheuristicに分類した。runtime/ledger観測のquorum判定は#51 reconcilerに残し、lintへdecision ruleを複製していない。
