# Issue #60 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: execution-topology review target、canonical review parser、CLI、reviewed compiler binding、schema/replay契約
- 調査モード: `intervention`
- 主要な結論: 専用targetは妥当。初期実装には、CLIだけでbytesを検証する境界漏れ、binding fieldの二重実装、観測revisionをdeployment baseにも使う時間境界の混同があった。いずれも実装中に修正した。
- 高確度候補数: 3（修正済み）
- 証拠上の制約: 実運用runtimeのtrace・組織コストはなく、構造、store E2E、Git履歴、既存revision規約を根拠とした。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- reviewerが観測したexact topology bytesだけがcompiler authorityになり、そのauthorityから作ったdeploymentが現行ledgerで実行可能であること。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | review morphism生成 | CLI、store replay、compiler、plan revision |
| `M` 評価指標 | hash一致・テスト成功 | authority非迂回、drift耐性、現行revisionでの利用可能性 |
| `N` 変更可能範囲 | `native_review` | compiler、CLI、schema、docs、E2Eを同時変更 |
| `T` 時間軸 | review command一回 | append後のcompile、reopen/reject、v0 replay |

制約はappend-only history、既存native review envelope v1のreplay互換、artifact cellのimmutability、LLM/runtime outputをaccepted truthにしないこと。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/native_review.rs:193-227,816-923`、`src/native_review/support.rs:59-130` | constructorとcanonical parser | 静的証拠 |
| 実行 | `tests/command.rs` のstore E2E、対象cargo test | proposal→artifact/claim→review→compile→reopen/reject | 実runtime dispatchは未実施 |
| 進化 | `git log -- src/native_review.rs src/graph_compiler.rs` | review/revision/gate変更履歴 | 共変更の定量分析なし |
| 意味・組織 | Issue #60、`docs/execution-topology-review.md` | reviewer authorityとmigration seam | チーム実測なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | observed revisionをdeployment baseにも流用 | review bindingが単純 | accept後のplanがstale | review関数→実行flow | 10 | C2 | `time-delayed`（修正済み） |
| 2 | bytes検証をCLIだけに置く | core APIが小さい | library callerが検証を迂回 | CLI→public library API | 12 | C2 | `externalization`（修正済み） |
| 3 | binding fieldをmetadata/parserへ個別列挙 | wire上見やすい | schema追加時にwriter/readerがdrift | module→contract lifecycle | 7 | C2 | `time-delayed`（修正済み） |

## 5. 上位候補の詳細

### Candidate I60-1: observed revisionとaccepted revisionの混同

#### 事実・推論・仮説

- [Evidence] targetは`observed_base_revision_id`を固定する（`src/native_review.rs:44-52`）。
- [Evidence] review morphismは別の`target_revision_id`へappendされる（`src/native_review.rs:193-227`）。
- [Evidence] plan系はcurrent revisionとの完全一致を要求する既存規約を持つ。
- [Inference] observed predecessorをcompiler request baseにすると、review acceptanceが存在するcurrent revisionではplanがstaleになる。
- [Hypothesis] 外部runtimeがhistorical baseだけを必要とする用途もあり得るが、native plan実行には適合しない。

#### 局所的合理性

- 局所目的: 「reviewerが何を見たか」を一つのrevision fieldで説明する。
- 直接の受益者: review contract実装者。
- 現在も有効な利益: observed revisionの固定自体は必須。
- 局所変更だけでは改善しにくい理由: compilerが必要とするのはacceptanceを含むrevisionで、概念が異なる。

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| observed revisionをcompile baseへ再利用 | accepted review append後にstale | callerがhistorical revisionを特別扱い | runtime adapter | reviewed compile毎 | compiler/plan revision invariant |

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | fieldが一つ | なし | 二revisionを区別 | 少し複雑 | 現在案 |
| モジュール | bindingが単純 | 意味が曖昧 | 意味が明示的 | record参照が必要 | 代替案 |
| 機能 | review証明可能 | compile結果がstale | acceptance存在revisionでcompile | なし | 代替案 |
| システム | なし | dispatch不能 | native revision規約と一致 | なし | 代替案 |
| 運用・組織 | なし | adapter特例 | 通常運用 | なし | 代替案 |
| ライフサイクル | 単純 | 特例が固定化 | v0で概念分離 | migration不要 | 代替案 |

- 反転する最小境界: compilerからplan/runtimeへ渡す機能境界
- 反転する指標: 実行可能性
- 反転する時間軸: review morphism append直後

#### 反実仮想

- A 現状維持: review証明は明瞭だが、caller側にstale補償が必要。
- B 局所改善: compiler requestのrevision比較を外す。revision authorityが弱まり不採用。
- C 構造変更: recordにはobserved predecessorを保持し、opaque compiler bindingのbaseにはreview morphism target revisionを用いる。append-onlyでrollbackはコード差し戻し可能。

#### スコア・判定

- `E=2, A=2, F=2, K=2, T=2`, Severity `10`, Confidence `C2`
- 分類: `time-delayed`
- 介入: `src/graph_compiler.rs`でaccepted target revisionをdeployment baseにし、E2E requestも同revisionで検証した。

### Candidate I60-2: CLI-only bytes verification

- [Evidence] CLIはinput bytesからraw artifact hashとcanonical topology hashを計算する。
- [Evidence] `execution_topology_review_morphism`はpublic APIであり、binding値だけを受ける設計ならCLIを通らず呼べる。
- 局所的利益: core API引数が小さい。
- 外部化コスト: library integratorが同じ検証を正しく再実装しない限り、caller-declared hashがauthority recordへ入る。
- 反転境界: binary CLIからpublic library APIへ拡張した時点。
- 反実仮想: A CLIだけ検証、B hashだけ再確認、C core constructorがartifact bytesをtyped parseしraw/canonical両hashとlineageを一括検証。
- `E=3, A=2, F=3, K=2, T=2`, Severity `12`, Confidence `C2`, `externalization`。
- 介入: Cを採用（`src/native_review.rs:193-198,816-923`）。CLIとlibrary callerが同一ruleを使う。

### Candidate I60-3: binding fieldのwriter/reader二重列挙

- [Evidence] targetはserde可能なclosed structである（`src/native_review.rs:42-53`）。
- [Evidence] 初期案ではwriterとcanonical parserが各field名を別々に列挙していた。
- 局所的利益: flat metadataは人間が読みやすい。
- 外部化コスト: optional field追加やrenameのたびにwriter/parser/schema/testを独立変更し、canonical authorityがdriftする。
- 反転境界: 同種contract変更が3回以上発生するlifecycle境界。
- 反実仮想: A flat重複、B helperで列挙共有、C `execution_topology_binding`をtyped serdeで一回serialize/deserialize。
- `E=1, A=2, F=1, K=1, T=2`, Severity `7`, Confidence `C2`, `time-delayed`。
- 介入: Cを採用（`src/native_review.rs:223-225`、`src/native_review/support.rs:105-120`）。

## 6. 横断的な補償構造

- 複数候補に共通する変換: caller input→claim metadata→review metadata→opaque compiler binding。
- 例外分岐: generic review APIは`ExecutionTopology`をfail closedし、専用constructorだけが生成可能。
- 再試行・手動運用: legacy plan/evidence reviewは自動昇格せず、明示的な再reviewが必要。
- 所有権・KPI: CLIの使いやすさだけでなくlibrary authority boundaryとruntime readinessを同時に評価した。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| native review envelope v1の維持 | nested v0との二version | additive target contractで既存review replayを壊さない | intentional compatibility boundary |
| accept/reject/reopenで同じartifact bytesを再提示 | 操作コスト | disposition対象が同じcontentであることをcoreで再検証する | audit safety |
| plan/evidence reviewをtopology review検索から除外 | 同一claimに複数review系列 | 別target kindはauthorityにも取消にもならない | bounded-context separation |

## 8. 未検証事項

- 大規模artifactを各dispositionで再読込する実測I/Oコスト。
- 外部MCP hostから専用CLI/APIを呼ぶ統合。
- experimental v0をstableへ上げる際の実runtime互換性。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | real runtime integration trace | accepted revisionからdispatchまで通るか | #58 pilot |
| 2 | 100MB以上のtopology artifact review計測 | bytes再検証コスト | benchmark fixture |
| 3 | experimental schema cross-contract CI | Rust serde/schema drift | #67 gate |

## 10. 介入判断の前提

- 変更範囲: native review、compiler、CLI、experimental schemaを同時変更可能。
- 移行期間: v0のためbreaking refinement可。ただしnative review v1 replayは維持。
- 一時的悪化: topology review commandはbytes再提示分だけI/Oが増える。
- 互換性制約: legacy plan/evidence reviewへauthorityを遡及付与しない。
- ロールバック要件: append済みv0 recordはunknown targetとしてfail closedし、履歴を書換えない。
