# Issue #62 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: `TopologyPatch`、canonicalization、deterministic apply/diff、proposal identity、review binding、`max_spawned_nodes` accounting。
- 主要な結論: proposal 件数を数える旧実装は controller 内では安価だったが、patch が追加する実 node 数を runtime/operator へ外部化する局所最適だった。computed diff による累積 accounting へ置換し、100-node patch を残予算 20 で全体 defer する実行証拠を得た。
- 高確度候補数: 1 件（修正済み）。typed patch を expansion module に置く局所性は、現時点では `harmless-locality` と判定した。
- 証拠上の制約: experimental v0 の実 runtime 運用履歴、巨大 patch の profile、複数チームの変更履歴はまだない。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- 動的 expansion が review seam を越えず、実際に追加される node 数、base topology、reviewed result を同じ canonical contract で拘束する。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` | candidate/controller | topology compiler、review、runtime/operator |
| `M` | proposal を生成できること | 実 node budget、再現性、substitution resistance、診断可能性 |
| `N` | counter の局所修正 | schema、Rust type、apply/diff、review check、tests の同時変更 |
| `T` | 1 round | 複数 round と v0 contract の進化 |

制約は Rust 1.80、proposal-only governance、既存 all-seen/dry/cost/latency semantics の維持である。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/dynamic_expansion.rs:128-425,650-945` | strict type、canonical maps、computed diff、atomic budget、exact reviewed hash | 静的証拠 |
| 実行 | `RUSTUP_TOOLCHAIN=1.80.0 cargo test --locked --test dynamic_expansion` | 11 tests pass。100 additions、duplicate、invalid removal、canonical identity、cumulative rounds | production load ではない |
| 実行 | unit test `accepted_review_cannot_substitute_a_different_topology` | exact patch application と異なる reviewed topology を拒否 | test-only opaque binding |
| 進化 | `git log -- src/dynamic_expansion.rs` | 元実装は `092fdcd` の experimental v0 のみ | 長期履歴なし |
| 意味・組織 | Issue #62 acceptance contract | budget は proposal count でなく cumulative actual additions | runtime 利用者観測なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | proposal ごとの `spawned_nodes += 1` | O(1) accounting | operator/runtime が hidden additions と budget overrun を負担 | 1 candidate が複数 node を追加 | 11 | C2 | `externalization`、修正済み |
| 2 | canonicalize と apply の二重 validation | public apply の安全性を再利用 | large patch の CPU/alloc と診断経路の重複 | 100+ node patch | 5 | C2 | `time-delayed`、監査中に修正 |
| 3 | patch type を expansion module に配置 | ownership と変更を一箇所に閉じる | 将来 compiler/redesign が同じ patch を必要とすれば依存方向が不自然 | 2 番目の実 consumer | 3 | C1 | `harmless-locality`（現時点） |

## 5. 上位候補の詳細

### Candidate EXP-COUNT-01

- 対象/所有: 旧 `ExpansionController::process_round` の proposal counter。
- 観測: 旧コードは candidate 1 件につき 1 を加算し、payload は任意 JSON だった。新コードは canonical patch の適用後 diff から `added_node_ids.len()` を得て、残予算超過時は proposal と counter の両方を変更しない（`src/dynamic_expansion.rs:688-748`）。
- 推論: 旧局所指標「accepted candidate 数」は policy 名 `max_spawned_nodes` と一致せず、budget の意味コストを runtime/operator へ押し出していた。
- 仮説: production で典型的な patch size が 1 なら発生頻度は低い。実 trace が必要。
- 局所的合理性: v0 scaffold では任意 runtime patch semantics を決めずに bounded loop を構築できた。

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| proposal count を node count とみなす | policy limit より多い node を review 可能 | reviewer が JSON を手で数える | reviewer/operator | patch size に比例 | Issue #62、100-node反例 |
| patch/result identity を比較しない | reviewed topology substitution | downstream hash 照合 | compiler/runtime host | review ごと | substitution unit test |

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 単純な加算 | semantic mismatch | diff 計算不要 | map 構築 | 旧案 |
| モジュール | loop 実装が小さい | patch validation 不在 | validation/apply が一貫 | 型とコード増 | 新案 |
| 機能 | 1-node patch は動く | multi-node で limit bypass | atomic refusal | invalid patch は reject | 新案 |
| システム | runtime 自由度 | review 内容と結果が未結合 | content-bound proposal/review | v0 contract coordination | 新案 |
| 運用・組織 | 初期実装が速い | reviewer が手作業補償 | policy 名と実測が一致 | schema 学習 | 新案 |
| ライフサイクル | 無形式 patch を変更しやすい | runtime ごとに dialect 化 | versioned schema evolution | breaking v0 migration | 新案 |

- 反転する最小境界: 1 candidate が 2 node 以上を追加する機能境界。
- `E=3, A=2, F=2, K=3, T=1`, Severity `11/15`, Confidence `C2`。
- 分類: `externalization`。反例 test と computed diff 実装により介入済み。

## 6. 横断的な補償構造

- 任意 JSON を runtime/reviewer が解釈する補償を、strict schema + Rust type に回収した。
- proposal と reviewed result の別々の hash 確認を、base hash + canonical patch の proposal identity と proposed topology hash の完全一致へ回収した。
- 監査で canonicalization が proposal path で二度走る構造を検出し、validated canonical patch 専用の private apply path に変更した。public apply は引き続き fail closed である。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| `BTreeMap` による全 node/edge 再構築 | large patch で O(n log n) | v0 に実測 bottleneck がなく、deterministic ordering と actual diff の利益が確認済み | correctness-first の experimental contract |
| proposal-only `accepted_graph_mutated: false` | acceptance まで自動化しない | CaseGraphen の authority boundary そのもの | 意図的な責務分離 |

## 8. 未検証事項

- 実 runtime が生成する patch size 分布と apply latency。
- compiler/redesign が同じ patch abstraction を必要とするか。
- `CandidateDecision` が単一 finding のみ返すことによる診断再試行コスト。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | runtime pilot の patch size/apply latency | map 再構築の実コスト | integration trace |
| 2 | invalid patch の finding 利用状況 | 単一 finding projection の負担 | operator feedback/telemetry |
| 3 | patch consumer inventory | module 抽出の時期 | Rust call-site audit |

## 10. 介入判断の前提

- v0 では breaking migration を許容するが、stable schema へ昇格する前に pilot trace を得る。
- module 抽出は第2の consumer が現れた時点で再評価する。
- rollback は typed patch consumer を旧 JSON へ戻すのではなく、v0 schema version を明示的に進める。
