# Issue #44 実装局所最適監査

## 1. エグゼクティブサマリー

- 調査範囲: `runtime.node_report.v0`、Rust validation/canonicalization、run completeness reconciliation、反例fixture。
- 主要な結論: discovery監査で2件の局所最適候補を確認し、どちらも実装中に修正した。未修正の重大候補はない。
- 高確度候補数: 2件（修正・回帰テスト済み）。
- 証拠上の制約: 新規実装のためGit共変更履歴、実runtimeのtrace、運用KPIは存在しない。進化・組織コストに関する判断はC1以下に留める。

## 2. システム成果と評価条件

最終成果は、外部runtimeの自己申告をCaseGraphenのtruthへ昇格させず、期待したgraphと実績の欠落・失敗・重複・artifact差分を再現可能に照合できることである。

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | node reportのparseと単一runの集計 | topology join、artifact ingest、将来の複数runtime adapter、CaseGraphen acceptance seam |
| `M` 評価指標 | schema妥当性とカウンタ値 | trust境界、診断の一意性、欠落をcompletionにしないこと、変更増幅 |
| `N` 変更可能範囲 | Issue #44の新規module/schema/test | #43 topology contractとの接続点まで。acceptance ruleやstoreは変更しない |
| `T` 時間軸 | experimental v0の初回実装 | 複数integrationで語彙を検証しstable contractへ昇格するまで |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/runtime_protocol.rs:1-6,353-429`、`schemas/experimental/README.md:3-19` | trust境界、join、artifact accounting | 静的証拠 |
| 実行 | `runtime_protocol` unit tests（9件） | 199/200、重複、retry、schema不一致、orphan artifact、graph mismatch | synthetic fixture |
| 進化 | stable/experimental schema directory分離 | v0変更の影響境界 | 新規実装のため履歴なし |
| 意味・組織 | Issue #44 trust rules、`CLAUDE.md` decision-rule/trust規則 | runtimeとacceptance kernelの責任分離 | 実運用者の観測なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | graph外reportによるartifact accounting（修正前） | 全reportを一回の走査で集計できる | graph joinを通らないruntime申告がorphan artifactを隠す | ingest/run境界 | 10 | C2 | `externalization`、修正済み |
| 2 | 診断ごとのduplicate counter加算（修正前） | 各分岐が独立して実装できる | 1 attemptが複数違反時にKPIを二重計上する | run report利用者 | 7 | C2 | `mixed`、修正済み |

## 5. 上位候補の詳細

### Candidate 1: graph外reportによるartifact accounting

- 観測: artifact claimの収集をgraph id/hash/node idの照合後へ置いた（`src/runtime_protocol.rs:400-428`）。graph外reportがorphan artifactをaccountできない回帰テストを追加した。
- 局所的合理性: 全reportを入力時に一括走査すれば短く、artifact claimの収集も安価である。
- 境界外コスト: 攻撃的または誤配送されたreportが、期待graphと無関係なartifactを名前だけで隠す。負担者はreconciler利用者と後続reviewerである。
- 補償ハロー: 後続integratorが再度graph membershipを検査するか、監査でartifactを手照合する必要が生じる。
- 反転: 関数内の走査量では修正前が僅かに単純だが、ingest/run境界ではjoin後だけaccountする案が優位。
- 反実仮想: A 現状維持はorphan見逃し、B 後段で再検査はdecision rule重複、C 同じreconciler内でjoin後に収集（採用）は追加移行なし。
- スコア: E3/A1/F2/K3/T1 = 10、Confidence C2、`externalization`。

### Candidate 2: duplicate counterの二重計上

- 観測: duplicate id、複数root、branched retry、invalid lineageは同じattemptへ重なり得る。現在は違反attempt indexの集合を単一sourceとして数える（`src/runtime_protocol.rs:384-399,431-507`）。
- 局所的合理性: 各validation分岐でカウンタを増やす方法は局所理解と追加が容易。
- 境界外コスト: finding数と重複attempt数が混同され、fleet KPIや再設計判断を過大化する。負担者はoperatorとaudit consumerである。
- 補償ハロー: consumer側dedupeまたは手動説明が必要になる。
- 反転: 個別分岐では直接加算が簡潔だが、run report境界ではoffender集合から一度だけ導出する案が優位。
- 反実仮想: A 直接加算維持、B 出力後に推測dedupe、C offender集合をdecision ruleにする（採用）。
- スコア: E2/A1/F1/K2/T1 = 7、Confidence C2、`mixed`。

## 6. 横断的な補償構造

2候補とも、局所分岐で集計を完結させると後続consumerへ再照合・dedupeを押し出す構造だった。修正後はgraph membershipとoffender identityをreconciler内部の単一導出へ集約した。acceptance、review、evidence attachmentは呼び出さず、別decision ruleを複製していない。

## 7. 優位性反転表

| 評価境界 | 修正前の利益 | 修正前のコスト | 採用案の利益 | 採用案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 分岐が短い | 意味の異なる値を同時集計 | join/offender集合が明示的 | 集合を保持 | ほぼ同等 |
| モジュール | 初期実装量が少ない | counter semanticsが分散 | 一つのreconcilerから導出 | 少量のmemory | 採用案 |
| 機能 | reportを早く処理 | orphan/duplicateが誤集計 | 反例を正しく拒否 | なし | 採用案 |
| システム | なし | downstream再検査 | consumerが同じ結果を得る | contract理解が必要 | 採用案 |
| 運用・組織 | なし | audit/KPIの手補正 | findingとcountの意味が分離 | 実測evalは今後 | 採用案 |
| ライフサイクル | 初期変更が少ない | adapterごとに補償が増える | v0で語彙を検証可能 | stable昇格時の移行 | 採用案 |

## 8. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| experimental schema directory | stable registryと別の契約置場 | v0語彙の変更をstable consumerへ波及させない明示境界であり、期限は実integration評価まで | #41/#43の段階導入と一致 |
| `RuntimeGraphExpectation` adapter型 | topology型との表現重複 | #43と独立した照合入力だが、graph hash/node/schemaの最小projectionに限定され、acceptance ruleを持たない | adapter境界として現時点では`harmless-locality` |
| reported model/contextを保持 | trust値の受入に見える | validation/completenessは値を権限判断に使わず、変更しても結果不変のテストがある | 観測metadataとして必要 |

## 9. 未検証事項と次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | #43 topologyの確定型とのadapter test | id/hash/node/schema projectionのdrift | #43統合後に直接変換testを追加 |
| 2 | 2種類以上のruntime JSONL | status/failure/resource語彙の不足 | #48 generic adapter fixture |
| 3 | 大規模run benchmark | BTree集合によるmemory/latency | 1k/10k/100k attempt fixture計測 |

## 10. 介入判断の前提

- `v0`はexperimental namespaceに留め、実runtime integrationの証拠なしにstableへ昇格しない。
- retry lineageはattempt idで明示し、時刻や配列順から推測しない。
- completenessは診断であり、evidence acceptanceやgoal achievementへ自動変換しない。
- rollbackはexperimental schema/moduleの利用停止で可能。stable schema/store migrationは発生しない。

## 11. 独立レビュー後の補正

初回監査後、`started_at`/`finished_at`を非空文字列としてしか検査しない局所最適を追加で検出した。node単体では文字列保持が最小だが、run/retry境界では不正な暦日や終了時刻の逆転がlatency・lineage監査へ補償処理を押し出す（E2/A1/F2/K2/T1 = 8、C2、`externalization`）。A: 非空検査のみ、B: 各adapterで時刻検査、C: runtime protocol境界でcanonical UTC形式・暦・順序を一度検査、を比較し、decision rule重複を避けるCを採用した。時刻は依然runtime自己申告であり、検査は形式整合性を保証するだけでworld anchorには昇格させない。
