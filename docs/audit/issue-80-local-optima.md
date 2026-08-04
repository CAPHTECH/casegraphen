# Issue #80 implementation local-optima audit

## 1. エグゼクティブサマリー

- 調査範囲: execution-topology acceptのsemantic/lint境界、CLI refusal、reject/reopen、後続compiler。
- 主要な結論: intrinsic validatorだけをaccept gateにする旧構造は関数内では単純だったが、canonical linterが既知のdeterministic graph errorをaccepted review後のcompilerへ外部化していた。review→compile機能境界で優位性が反転する `externalization` だった。
- 高確度候補数: 1。
- 証拠上の制約: production運用の再レビュー回数・待ち時間は未計測。静的構造と決定論的fixtureによる実行証拠を使用した。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- content-bound topology reviewのacceptが、同じcanonical contractではdeploy不能と既知のtopologyへauthorityを付与しない。
- heuristic adviceをauthority ruleへ昇格せず、invalid proposalのreject/reopen履歴を維持する。
- CLI/host consumerがlint code、location、detailをprose解析せず扱える。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | `execution_topology_review_morphism` | review、CLI refusal、compiler、append-only lifecycle |
| `M` 評価指標 | constructorの単純さ、semantic validity | accepted authorityのdeployability、再レビュー回数、rule drift |
| `N` 変更可能範囲 | review moduleのみ | canonical linter型、review error projection、docs/tests |
| `T` 時間軸 | accept呼出し一回 | reject/reopen、reviewed compile、将来lint rule追加 |

制約はappend-only history、canonical decision ruleの単一所有、heuristic adviceの非authority性、既存review envelope互換である。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/native_review.rs`, `src/graph_lint.rs`, `src/graph_compiler.rs` | validator/linter/accept/compilerの呼出し境界 | 静的証拠 |
| 実行 | `src/native_review/tests/mod.rs`, `src/native_cli/tests.rs`, `tests/command.rs` | cycle、contract error、mixed、warning、advisory、reviewed compile | synthetic fixture |
| 進化 | Issue #75 audit、Issue #80 | semantic validation追加後もcycleが残った変更増幅 | PR待ち時間は未計測 |
| 意味・組織 | ADR 0023、execution-topology review guide | review authorityとaudit adviceの語彙 | 組織KPIは未計測 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | accept gateをintrinsic validatorだけに限定 | review関数がgraph analysisを知らず単純 | accepted-but-undeployable record、compiler refusal、再review | review→compile機能境界 | 7 | C2 | externalization |

## 5. 上位候補の詳細

### 5.1 識別

- Candidate ID: I80-C1
- 名称: topology graph-shape validityのcompiler先送り
- 対象実装: `require_execution_topology_review_target`
- 所有モジュール / サービス: `native_review`, `graph_lint`, `graph_compiler`
- 所有チーム: CaseGraphen maintainers
- 導入時期: Issue #75 semantic hardening
- 調査者: implementation agent

### 5.2 事実・推論・仮説

#### 観測された事実

- [Evidence] `graph_lint::lint_execution_topology`はintrinsic validationを`contract_*` deterministic errorへ写像し、cycleを`dependency_cycle` deterministic errorとして同じtyped reportへ集約する。
- [Evidence] 旧review pathは`validate_execution_topology`だけをaccept blockerにし、lintからはheuristicだけを後から抽出した。
- [Evidence] compiler/runtime/expansionはすでに `Deterministic && Error` をblockerとしてcanonical lintから選択する。
- [Evidence] cycle fixtureはsemantic parseを通る一方、canonical lintにdeterministic errorとheuristic adviceが共存する。

#### 推論

- [Inference] reviewがcycleをacceptしcompilerが同じcanonical topologyを拒否すると、reviewerとdeployment operatorが再レビュー負担を負う。
- [Inference] review moduleへcycle traversalを複製せず、canonical lintのpublished classification/severityを選択する方が将来rule追加時のdriftを抑える。

#### 未検証仮説

- [Hypothesis] production pilotでaccepted後compiler refusalが発生した頻度と平均再レビュー時間。
- [Hypothesis] hostが将来ledger mutationを委譲する際のstructured finding consumer互換。

### 5.3 局所的合理性

- 局所目的: content/semantic bindingを小さいreview constructorで保証する。
- 局所指標: validation branch数、review moduleのgraph algorithm依存。
- 直接の受益者: review module maintainer。
- 現在得られていた利益: invalid reference/data bindingは早期拒否し、heuristic adviceは非blockingだった。
- 導入時の制約: deterministic validatorとheuristic linterを混同しないこと。
- 現在も有効な制約: heuristicはauthorityにならず、decision ruleを複製しないこと。
- 失効した制約: linter全体をaccept gateに使うとheuristicまでblockする、という前提。reportはclassification/severityをtypedに分離済みである。

### 5.4 評価条件

- `B`: review constructorからreview→compile lifecycleへ拡張。
- `M`: 実装量からauthority/deployability整合と変更増幅へ拡張。
- `N`: validatorかreviewの一方だけでなくcanonical lint/error projectionを同時変更。
- `T`: 今回のacceptから将来rule追加・再レビューまで拡張。

### 5.5 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| reviewはintrinsic validatorだけを見る | cycleへaccepted reviewが作れる | compilerが後段拒否 | deployment operator | cycle proposalごと | compiler lint gate、cycle fixture |
| lintをadvisory収集時に再実行 | accept判定とmetadata生成が別pass | 両passの同一性を暗黙に信頼 | maintainer/reviewer | acceptごと | 旧`native_review.rs` call sites |
| refusalがmessageのみ | consumerがcode/path/detailをparse | CLI agent固有prose parsing | CLI/host integrator | refusalごと | 旧`NativeReviewError` shape |

### 5.6 四観測面の証拠

#### 構造

- canonical lintはvalidatorとgraph-shape analysisを一つのsorted reportへ集約済みであり、review側にrule複製は不要。
- 修正後はlintを一度実行し、deterministic error、heuristic advisoryを同じreportから分岐する。

#### 実行

- testsはdependency cycle、contract errors、mixed classes、deterministic warning、heuristic-only acceptance、reject/reopen、reviewed compileを観測する。
- CLI refusal projectionはtyped findingのcode/location/detailをJSON `data.findings`へ保持する。

#### 進化

- Issue #75はintrinsic semantic gapを閉じたが、graph-shape ruleを別gateに残したためIssue #80が必要になった。decision report全体を境界にすることで同種のrule-by-rule追随を避ける。

#### 意味・組織

- `deterministic/error`はauthority blocker、`heuristic`はreview adviceという語彙をADRとguideで固定した。warningはclassificationだけを理由に昇格しない。

### 5.7 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | validator callが明快 | lintを後で再実行 | lint一回で分類 | typed error追加 | 旧案僅差 |
| モジュール | semanticとauditを表面上分離 | 同じtopologyを二pass | reportの型境界を再利用 | graph_lint依存 | 新案 |
| 機能 | intrinsic invalidityは拒否 | cycleをaccept後拒否 | accepted authorityとdeployability整合 | reject/reopen分岐維持 | 新案 |
| システム | compilerが最終防御 | accepted-but-undeployable state | review seamで早期拒否 | structured payload互換 | 新案 |
| 運用・組織 | reviewerはsemanticだけ判断 | operatorが再reviewを調整 | 一回の全blocker提示 | reviewerがdeterministic errorsを先に直す | 新案 |
| ライフサイクル | rule追加時もcompilerは安全 | review gateがrule追加に追随しない | canonical classification追加が自動反映 | classification contract管理 | 新案 |

- 反転する最小境界: review→compilerの機能境界。
- 反転する指標: constructor単純さからaccepted authorityのdeployabilityへ。
- 反転する時間軸: accept直後からreviewed compile時。

### 5.8 反実仮想

#### A. 現状維持

- 定常コスト: compilerを最終防御として維持。
- 将来コスト: 新しいdeterministic graph ruleごとにaccepted-but-refused状態が増える。
- リスク: append-only historyへ実行不能なaccept authorityが残る。

#### B. 最小限の局所改善

- 変更: review側へcycle検出だけを追加。
- 利益: 現在のcycle fixtureを拒否。
- 残る問題: resource等の次のgraph ruleで再発しdecision ruleがdriftする。
- 移行コスト: 小さいが継続的な二重保守が必要。

#### C. 境界をまたぐ構造変更

- 変更: canonical lintを一度呼び、`Deterministic && Error`だけblock、heuristicだけreview metadataへ保持、typed findingsをCLIへ投影する。
- 成立条件: graph lint classification/severityがversioned contractであること。
- 定常利益: graph ruleの単一所有、早期拒否、structured recovery。
- 新たなコスト: `NativeReviewError`がgraph finding型を運ぶ結合。
- 移行の谷: tests/docs/error consumersを同時更新する。
- ロールバック: lint selectorを外せば旧挙動へ戻せるが、作成済みreview historyは改変しない。

### 5.9 スコア

- `E` 外部化コスト: 2
- `A` 変更増幅: 2
- `F` 境界障害: 1
- `K` KPI乖離: 1
- `T` 時間ロックイン: 1
- `Severity`: 7 / 15
- `Confidence`: C2

### 5.10 判定

- 分類: `externalization`
- 判定理由: 局所的なconstructor単純化が、既知のdeterministic refusalと再レビューをcompiler/operatorへ押し出し、機能境界で優位性が反転した。
- 反証となり得る情報: accepted topologyはdeployment authorityではなく、deploy不能acceptを意図した独立概念であるというcontract。ただしADR 0023はreviewed compilation authorityを明示するため該当しない。
- 未検証事項: production頻度、host mutation delegateの将来shape。
- 次に取得すべき証拠: runtime pilotでreview refusal→修正→accept→compileの所要時間とfinding consumer telemetry。

## 6. 横断的な補償構造

- 複数候補に共通する変換: validator finding→lint finding→prose errorの変換。
- 複数候補に共通する例外分岐: acceptだけblockしreject/reopenはidentity/content検証のみ行う。
- 複数候補に共通する再試行・手動運用: compiler拒否後のtopology修正と再review。
- 所有権・KPIに起因する再発構造: reviewの小ささとend-to-end authority整合を別指標にするとvalidation ruleの先送りが再発する。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| heuristic findingの非blocking | warningをacceptする | 観測強度上、authority errorではなくreviewer判断であり反転しない | intentional trust boundary |
| reject/reopenのlint bypass | invalid graphをmorphism化する | acceptanceではなくexact invalid artifactへのdispositionをappend-onlyに記録する | auditability requirement |
| compilerのlint再検証 | reviewと重複 | stored review/bytes改竄や直接library callerに対するdefense-in-depthで負担者が同じ | security boundary |

## 8. 未検証事項

- operational MCP hostは現在acceptance-ledger mutationを委譲しないため、実host経由のtopology-review refusalは存在しない。
- 長期運用でのaccepted-but-undeployable record発生率は取得していない。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | full topology-review CLI JSON refusal | structured payloadのend-to-end wire保持 | cycle fixture storeでCLIを実行 |
| 2 | independent host consumer | 将来host delegateのtyped finding保持 | mutation delegate追加時のcontract test |
| 3 | pilot review/compile timings | operational severity | retained pilot traceを比較 |

## 10. 介入判断の前提

- 変更可能なチーム・サービス範囲: experimental graph-lint/review/CLI contract。
- 許容できる移行期間: v0期間中に同時更新。
- 一時的に悪化してよい指標: error payload size。
- 互換性・規制・SLO制約: existing review envelopeとappend-only historyを維持。
- ロールバック要件: historyを書き換えずcode selectorのみ戻せること。

## 11. 実装後の判定

候補Cを採用した。review moduleはcanonical linterのtyped結果を選択するだけで、cycle、reachability、resource等のruleを持たない。deterministic warningsはblockせず、heuristic findingsはaccepted reviewのadvisoryとして保持する。reject/reopenも維持される。調査境界内で残るmaterialな局所最適は確認していない。
