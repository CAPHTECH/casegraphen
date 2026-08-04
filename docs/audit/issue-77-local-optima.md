# 実装局所最適監査レポート — Issue 77

## 1. エグゼクティブサマリー

- 調査範囲: `RuntimeGraphExpectation`、terminal retry選択、artifact ingest、generic JSONL、streaming early release、experimental schema。
- 主要な結論: 旧node-only completenessは小さなruntime-neutral contractとして合理的だったが、execution topology境界まで広げるとdataflow未実行を`complete`と呼ぶコストをadapter、reviewer、promotion判断へ外部化する局所最適だった。canonical topology projectionとedge proofへ置換した。
- 高確度候補数: 1（修正済み）。
- 証拠上の制約: 実行証拠は決定論的fixtureとgeneric JSONL integrationであり、remote runtimeのp95/p99、巨大artifact、長期retry履歴は未計測。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- `complete`が「全node reportがある」ではなく「review済みの設計どおりにartifactがedgeを流れた」ことを示し、なおruntime claimをaccepted truthへ昇格させない。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | node report集計 | topology → retry attempts → edge handoff → ingest bytes → streaming/review seam |
| `M` 評価指標 | expected nodeのterminal success率 | graph fidelity、欠落/代替検出、診断可能性、decision-rule一意性 |
| `N` 変更可能範囲 | `runtime_protocol`のみ | protocol、integration、streaming、schema、Skill、pilot documentation |
| `T` 時間軸 | 一回のrun reconciliation | 複数runtime adapterとstable promotion後の契約進化 |

制約はRust 1.80、experimental v0の破壊的変更許容、append-only acceptance ledgerとのtrust seam、runtime schedulerをcoreへ持ち込まないこと。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/runtime_protocol.rs:119-327,570-932` | canonical expectation、opaque byte observation、terminal/edge proof | 静的証拠 |
| 実行 | `tests/runtime_edge_reconciliation.rs:195-390` | fan-out/reduce、retry replacement、missing/substitution/duplicate/un-ingested、parent/schema、JSONL review halt | fixture、非負荷試験 |
| 進化 | commit `55fd682`、`docs/audit/issue-44-local-optima.md:79-88`、`issue-48-local-optima.md:42-50` | node projectionとadapter output-byte補償の段階的拡張 | PR待ち時間等は未取得 |
| 意味・組織 | `docs/adr/0023-runtime-completeness-requires-edge-handoffs.md`、integrate Skill | completeness所有者とruntime/reviewer境界 | 実組織KPIなし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | node-only `RuntimeGraphExpectation` / `complete` | 小さくruntime-neutral、retry/schema/missing検査を一箇所に保持 | edgeを流していない成功node集合の検出をadapter・reviewer・pilotへ押し出す | node集計 → execution topology/run graph | 12 | C3 | `externalization`、修正済み |

## 5. 上位候補の詳細 — LO-77-1

### 5.1 識別

- Candidate ID: LO-77-1
- 名称: node成功をgraph完了と同一視するcompleteness境界
- 対象実装: 旧`RuntimeGraphExpectation.nodes`と`reconcile_runtime_reports`
- 所有モジュール: `runtime_protocol`、境界consumerは`runtime_integration`と`streaming_reconciliation`
- 導入時期: `092fdcd` / `55fd682`系列
- 調査者: Codex issue-77 lane

### 5.2 事実・推論・仮説

#### 観測された事実

- [Evidence] 旧監査はnode/schemaだけのprojectionをadapter境界として合理的と判定し、topology確定後のdrift testを未検証としていた（`docs/audit/issue-44-local-optima.md:79-88`）。
- [Evidence] generic JSONLはprotocol外で「宣言output bytesがinventoryにあるか」を補償していた（`docs/audit/issue-48-local-optima.md:42-50`）。
- [Evidence] 新反例ではnode completenessがtrueでも一つのreduce inputを除くとdataflow completenessがfalseになる（`tests/runtime_edge_reconciliation.rs:241-257`）。
- [Evidence] canonical retry terminalのartifactだけが二つのoutgoing edgeを満たす（同`:293-325`）。
- [Evidence] valid JSONL graphはedge proof後も`accepted:false`、`needs_review`で停止する（同`:357-390`）。

#### 推論

- [Inference] node-only設計は`runtime_protocol`単体の実装量を減らしたが、「設計したgraphが流れたか」という意味コストをadapter固有検査と人手reviewへ移していた。
- [Inference] topologyからexpectationを一度だけ導出しbatch/streamingが同じterminal projectionを使うと、adapter追加時のdecision-rule複製を抑えられる。

#### 未検証仮説

- [Hypothesis] 10万edgeでの集合intersection、proof serialization、全report保持が許容memory/p99に収まる。
- [Hypothesis] chunked binary artifactのtransport observationを追加しても同じcontent proofを維持できる。

### 5.3 局所的合理性

- 局所目的: runtime reportの欠落、retry、schema、graph joinを小さなruntime-neutral型で判定する。
- 局所指標: contract field数、adapter実装量、node report completeness。
- 直接の受益者: protocolと初期adapterの実装者。
- 現在得られていた利益: retry lineageとnode schemaのcanonical判定は一箇所に保たれていた。
- 導入時の制約: execution topology語彙と実runtime pilotが未成熟。
- 現在も有効な制約: runtime observationはuntrusted、acceptanceは別review、schedulerは外部所有。
- 失効した制約: topologyにtyped data edge/output/input/schema/deliveryがないという制約。

### 5.4 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| expectationはnode/schemaのみ | handoff欠落をprotocolで検出不能 | adapterのoutput-byte存在検査 | runtime adapter owner | adapterごと/runごと | issue-48監査、`runtime_integration`旧境界 |
| `complete`をnode結果で計算 | graphを流さない独立実行も完了表示可能 | reviewer/pilotがinput/outputを目視 | reviewer/operator | data-edgeを持つ全run | issue 77反例 |
| streamingがevent edgeだけ検査 | superseded attemptや未ingest bytesからrelease可能 | streaming固有条件追加 | streaming maintainer | streaming eventごと | `src/streaming_reconciliation.rs:335-353,502-571` |

### 5.5 四観測面の証拠

- 構造: canonical topology projectionがnodes、parents、data edgesを一度に生成する（`runtime_protocol.rs:232-304`）。artifact observationはbytes提示なしにfield constructionできない（同`:154-178,306-327`）。
- 実行: 6 edge tests、4 integration tests、10 streaming tests、3 experimental schema conformance testsが通過。
- 進化: node-only projection → adapter-local output-byte補償 → edge-level canonical proofという変更系列が確認できる。
- 意味・組織: ADR 0023はruntime observationとacceptanceの所有者を統合せず、`complete`の意味だけをgraph境界へ拡張する。

### 5.6 境界拡張と優位性反転

| 評価境界 | 現在案（旧node-only）の利益 | 現在案のコスト | 代替案（edge proof）の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 少ないfieldと集合 | handoffを観測しない | 判定は増える | intersection/proof生成 | 旧案 |
| モジュール | retry/schemaが一箇所 | artifact bytesは外部 | terminal選択とedge proofを同居 | topology型依存 | 代替案 |
| 機能 | 全node成功を高速表示 | fan-out/reduceの欠落を誤表示 | node/edgeを別診断 | runtime report整備 | 代替案 |
| システム | adapterが独自補償可能 | adapter間で意味がdrift | JSONL/streaming共通意味 | v0 contract変更 | 代替案 |
| 運用・組織 | protocol ownerの範囲が小さい | reviewer/operatorへ目視負担 | stable findingとproof | proof理解が必要 | 代替案 |
| ライフサイクル | 初期変更が少ない | adapterごと補償が固定化 | 新runtimeも同じprojection | 大規模性能検証が必要 | 代替案 |

- 反転する最小境界: `runtime_protocol`関数からgeneric runtime integration機能へ広げた時点。
- 反転する指標: node report completenessからdesigned dataflow completeness。
- 反転する時間軸: 二つ目のconsumer（streaming）が同じ補償を必要とした時点。

### 5.7 反実仮想

#### A. 現状維持

- 定常コスト: adapterとreviewerがinput/output joinを個別確認。
- 将来コスト: runtime familyごとに`complete`の意味が分岐。
- リスク: silent handoff omission、promotion evidenceの過大評価。

#### B. 最小限の局所改善

- 変更: generic JSONLだけでsource/target artifact intersectionを検査。
- 利益: 現在の主要adapterを修正可能。
- 残る問題: streaming、Rust caller、次adapterが別ruleを持つ。
- 移行コスト: 小さいが意味分岐を固定化する。

#### C. 境界をまたぐ構造変更（採用）

- 変更: topology-derived expectation、opaque byte observation、canonical terminal/edge proof、node/dataflow別結果をprotocolに置き、integration/streamingから委譲。
- 成立条件: topology semantic validation、content-addressed artifact ingest。
- 定常利益: graph completenessの一意な意味とstable findings。
- 新たなコスト: topology module依存、result/schema増加、edge集合処理。
- 移行の谷: historical pilot reportはedge evidenceとして再利用不能。
- ロールバック: experimental v0なのでschema/consumerを旧版へ戻せるが、node-only `complete`をpromotion根拠にしてはならない。

### 5.8 スコアと判定

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 3
- `K` KPI乖離: 3
- `T` 時間ロックイン: 1
- `Severity`: 12 / 15
- `Confidence`: C3（反例実行と変更系列が一致）
- 分類: `externalization`、修正済み。
- 反証となり得る情報: 全consumerがedgeのない独立node集合だけを扱うなら旧案の優位性は反転しない。しかしcanonical topologyとpilotはdata edgeを持つため現状には該当しない。

## 6. 横断的な補償構造

- 変換: topology → runtime expectationを各adapterで作らず、`derive_runtime_graph_expectation`へ集約した。
- 例外分岐: node output cardinality v0制約はprojectionで一度だけ拒否する。
- 再試行・手動運用: retry順序やterminal artifactをJSONL順・operator判断から推論しない。
- 所有権: runtimeはreport/bytes、protocolはdeterministic reconciliation、CaseGraphen reviewはacceptanceを所有する。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| `ExpectedRuntimeEdge`がtopology edgeを複製 | 表現重複 | runtime deployment hashに結合した最小projectionで、別decision ruleを持たずcanonical constructorが唯一の製品経路 | runtime-neutral adapter境界として`harmless-locality` |
| `node_complete`を残す | 旧KPI温存 | `complete` authorityには使わず、missing nodeとmissing edgeの診断分離に必要 | compatibility/diagnosisの意図的重複 |
| 同じartifactをfan-outの複数edgeで使う | duplicateに見える | 一つのimmutable content addressを複数consumerが読む正当なdataflow | data replicationを伴わない安全な共有 |

## 8. 未検証事項

- 10万edge/attemptでのmemory、p95/p99、proof payload size。
- binary/chunk aggregate bytesとartifact IDのend-to-end transport proof。
- historical issue-76 pilotの新contractによる再実行。
- runtime clock/actor/modelの真実性（本変更はcontent/dataflow proofだけを扱う）。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | issue-76四runtime family再実行 | adapterごとのinput/output/parent整合性 | pilot scriptを新hostで実行しretained manifest更新 |
| 2 | 1k/10k/100k edge benchmark | BTree集合とproof serializationの増幅 | fixed seed DAG benchmark、p50/p95/p99/RSS |
| 3 | binary/chunk crash-resume pilot | streaming chunkとfinal artifact bytesのatomicity | content-addressed binary fixtureと中断再開 |

## 10. 介入判断の前提

- 変更可能範囲: experimental protocol、adapter、streaming、schema、Skill。
- 許容移行期間: v0の間。stable consumerはまだない。
- 一時的に悪化してよい指標: result bytes、reconciliation CPU、historical report互換性。
- 互換性・SLO制約: Rust 1.80、deterministic findings、runtime outputはuntrusted、acceptedは常にreview所有。
- ロールバック要件: contract rollback時もedge evidenceなしの`complete`をstable promotionへ使用しない。
