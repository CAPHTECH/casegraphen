# 実装局所最適監査レポート — Issue 82

## 1. エグゼクティブサマリー

- 調査範囲: `docs/adr/`の採番・ファイル名・heading・Markdown参照、READMEのnext-ID表示、release quality gate、negative fixtures。
- 主要な結論: 手動採番はADR一枚を追加する局所ではコストが低い。しかしdecision identityを使う実装・Skills・監査・運用まで境界を広げると、重複と参照の曖昧性を各consumerへ外部化する。連続inventoryと決定論的gateへ置換した。
- 高確度候補数: 1件（修正済み）。
- 証拠上の制約: 構造証拠、Git履歴、実行したpositive/negative conformanceはあるが、CI待ち時間やreviewerの誤読件数は計測していない。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- `ADR NNNN`がrepository全体で一つのdecisionを指し、参照先の欠落や別decisionへの無言再利用をrelease前に決定論的に拒否する。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | ADR一枚のfilename/heading | ADR inventory → repository Markdown → Skills/guides/audits → CI/release |
| `M` 評価指標 | 追加の容易さ | identity一意性、参照完全性、失敗診断、将来の変更増幅 |
| `N` 変更範囲 | 対象Markdownだけ | ADR改番、全参照、checker、fixtures、static analysis、README |
| `T` 時間軸 | 今回の文書追加 | 同種ADR追加が3回・10回続く製品寿命 |

制約はPython標準ライブラリだけで実行できること、CIが`sh scripts/static-analysis.sh`だけを実行すること、過去decisionの本文と意味を変えないことである。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `docs/adr/`、`scripts/adr-conformance.py`、README | 25 ADRのID・heading・link・next ID | Markdownのinline/reference linkが対象、HTML linkは対象外 |
| 実行 | `python3 scripts/adr-conformance.py --index README.md`、`tests/adr_conformance.rs`、4 fixture | positive inventoryとduplicate/mismatch/missing/broken-link refusal | local filesystem、remote CIはroot laneの統合後証拠 |
| 進化 | `git log -- docs/adr scripts/static-analysis.sh`、Issue #82 | ADRが複数commitで追加され0020/0023が再利用された履歴 | PR滞留時間なし |
| 意味・組織 | ADR 0012、過去のIssue 53/77 audit、README document index | decision identityの利用者と更新責任 | 実組織の所有者別コストは未観測 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | ADR採番と参照の手動管理 | 追加時にchecker/inventory更新が不要 | consumerが別decisionを誤読し、将来の修正者が全参照を手作業で探す | 一文書 → repository/release | 10 | C2 | `time-delayed`／修正済み |
| 2 | repository全Markdownの毎回scan | 別indexを持たずsourceを直接確認 | repository成長時のCI I/O | 現在のrepository → 大規模monorepo | 3 | C1 | `insufficient-evidence` |

## 5. 上位候補の詳細 — LO-82-1

### 5.1 識別

- Candidate ID: LO-82-1
- 名称: ADR decision identityの手動採番
- 対象実装: 旧`docs/adr/` directoryとrelease gate
- 所有モジュール: repository documentation / release quality
- 導入時期: ADR 0001導入時からIssue #82まで
- 調査者: Codex issue-82 lane

### 5.2 事実・推論・仮説

#### 観測された事実

- [Evidence] 二つのfileがheadingとfilenameの両方で0020を使い、別の二つが0023を使っていた（Issue #82と変更前tree）。
- [Evidence] `scripts/static-analysis.sh`はSkill/product/schemaを検査したがADR inventoryは検査していなかった。
- [Evidence] `docs/audit/issue-53-local-optima.md`の「ADR 0020」はstreaming decision、READMEとADR 0019の「ADR 0020」はproduct surfaceを意味していた。
- [Evidence] 新checkerは25件を順序に依存せずinventory化し、四つのnegative fixtureをそれぞれrefuseした。

#### 推論

- [Inference] 文書一枚の所有者は採番コストを節約するが、利用者は文脈なしのprose referenceを一意に復元できない。
- [Inference] 追加時に一度だけgateを通すコストは、複数consumerが各回目視確認する将来コストよりrepository/release境界で小さい。

#### 未検証仮説

- [Hypothesis] Markdown数が10倍になった場合でも全scanのCI時間は無視できる。現在は実測なし。
- [Hypothesis] proseだけの`ADR NNNN`の意味的参照先まで機械検査する追加価値がある。ID存在確認だけでは意味の取り違えを証明できない。

### 5.3 局所的合理性

- 局所目的: ADRをMarkdown一枚で追加し、他toolやmanifestを不要にする。
- 局所指標: 追加file数とauthoring手順の少なさ。
- 直接の受益者: ADR author。
- 現在も有効な利益: ADR本文は通常のMarkdownで読め、専用generatorは不要。
- 失効した制約: ADR数が少なく、参照consumerがREADMEとコードに限られるという暗黙条件。
- 局所変更だけでは改善しにくい理由: duplicateの改番だけでは次回の再利用を防げず、release gateとnegative counterexampleが必要。

### 5.4 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| next IDをauthorの記憶で選ぶ | duplicate/gap | directoryを手作業で列挙 | 次のauthor/reviewer | ADR追加ごと | duplicate 0020/0023、missing 0012 |
| filenameとheadingを別入力 | identity drift | reviewで二箇所を目視 | reviewer | renameごと | Issue #82 acceptance |
| link生存をconsumerへ委譲 | rename後のbroken link | `rg`と手動修正 | Skills/guides/audit所有者 | 改番ごと | Issue 53/77 audit参照の同時変更 |

### 5.5 四観測面

- 構造: filename、heading、prose、linkが同じIDを別々に保持し、旧gateにjoinはなかった。
- 実行: positive inventoryは`ok (25 ADRs, next 0026)`、4 fixtureは各refusalの診断を生成した。
- 進化: ADR群は2026-07-30から2026-08-04の複数commitで増加し、Graph Engineering Plane追加後にduplicateが発生した。
- 意味・組織: authorの局所的利益に対し、コストはreviewerと後続decision consumerが負担していた。

### 5.6 境界拡張と優位性反転

| 評価境界 | 手動管理の利益 | 手動管理のコスト | gateの利益 | gateのコスト | 優位性 |
|---|---|---|---|---|---|
| 関数/一file | tool不要 | 二重入力 | 入力診断 | script実行 | 手動がやや有利 |
| モジュール/ADR directory | 構成が自由 | duplicate/gapを見落とす | inventory一意性 | 連続番号の制約 | gateが有利 |
| 機能/documentation | 個別文書だけ変更 | link/proseの修正漏れ | broken linkをrelease前拒否 | scan I/O | gateが有利 |
| システム/release | CI step一つが少ない | 別decisionを同じIDで出荷 | 他gateと同じfail-closed semantics | 数秒未満のPython実行（現時点） | gateが有利 |
| 運用・組織 | author自律 | reviewerが毎回全履歴を確認 | 責任をrepositoryに固定 | convention学習 | gateが有利 |
| ライフサイクル | 削除/再利用が容易 | identityの過去意味を破壊 | tombstone/status recordで意味保存 | 欠番を作れない | gateが有利 |

- 反転する最小境界: ADR directory inventory。
- 反転する指標: authoring file数からdecision identity一意性へ変えたとき。
- 反転する時間軸: duplicateが後続consumerに参照される次の変更時。

### 5.7 反実仮想

#### A. 現状維持

- 定常コスト: 追加はMarkdown一枚。
- 将来コスト: 重複と欠落の全参照を人手で監査。
- リスク: リンクが解決してもprose identityが別decisionを指す。

#### B. 最小限の局所改善

- 変更: duplicateの四fileだけをrename。
- 利益: 現在の曖昧性は消える。
- 残る問題: 次のADR追加で再発、broken linkは未検出。
- 移行コスト: 小、rollbackはrenameを戻すだけ。

#### C. 境界をまたぐ構造変更（実装案）

- 変更: 改番+連続inventory+filename/heading/link/next-ID checker+negative fixtures+release gate。
- 成立条件: ファイルは四桁連番、削除したdecisionはstatus recordとして残す。
- 定常利益: 同種の失敗をCIで再現可能に拒否。
- 新たなコスト: README next-IDをADR追加と同時更新、全Markdown scan。
- 移行の谷: 旧ファイル名を直接参照するrepository外consumerは同時変更できない。リポジトリ内は一括変更で緩和。
- ロールバック: renameとgateの同時revertは可能だが、重複identityを再導入するため通常は選ばない。

### 5.8 スコアと判定

- `E=2`: 後続のauthor/reviewer/audit consumerへ継続負担。
- `A=3`: renameがSkills・guides・audits・READMEへ波及。
- `F=1`: runtime障害ではないがdecision参照境界で誤読。
- `K=2`: 「Markdownが追加できた」と「decisionが一意に監査できる」が乖離。
- `T=2`: 後続参照が増えるほど改番コストが固定化。
- `Severity=10`、`Confidence=C2`。
- 分類: `time-delayed`（修正済み）。
- 判定理由: 局所利益は存在するが、構造・進化面の独立証拠とnegative executionがrepository境界で優位性反転を示す。
- 反証となり得る情報: ADR番号は参照identityではなくfilenameだけがidentityであるという正式contract。現実のcode/docsは`ADR NNNN`を直接参照するため該当しない。

## 6. 横断的な補償構造

- 変換: filename prefixとheading IDの人手join。
- 例外分岐: 欠番とduplicateを「このADRだけ」と説明する暗黙allowlist。
- 手動運用: `rg`で旧パスとproseを探し、判断ごとにどちらのADRか読み分ける。
- 再発構造: next IDをrepositoryが示さず、reviewerだけが全体一意性を所有する。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| ADR 0012で連続採番を強制 | tombstone文書が将来増える | decision identityの再利用防止が目的で、status recordを残すコストは監査可能性の一部 | `harmless-locality` |
| Python標準ライブラリのchecker | Markdown parserを完全実装していない | ADR linkとしてrepositoryが使うinline/reference destinationを検査し、新dependencyを追加しない | 調査範囲では合理的 |

## 8. 未検証事項

- repository外から旧ADRパスを参照するlinkの有無。
- 大規模repositoryで`rglob("*.md")`がCI p95に与える影響。
- CommonMarkのHTML linkや動的に生成されるlinkは検査対象外。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | remote CIのADR gate result | localとCIの実行等価性 | root laneがpush後にquality workflowを保存 |
| 2 | 1万Markdownでのscan benchmark | time-delayed CI cost | temp fixtureを生成しwall time/RSSを計測 |
| 3 | repository外link search | renameの移行の谷 | GitHub code searchまたはconsumer inventory |

## 10. 介入判断の前提

- 変更可能範囲: repository内ADR、docs、Skills、tests、release gateは同時変更可能。
- 移行期間: 一commitでrenameと参照更新をatomicに行う。
- 一時的に悪化してよい指標: authoring手順とCI step数。
- 互換性制約: 過去ADR本文のdecision contentは変えない。
- ロールバック要件: checker単体はrevert可能だが、一意な新IDは公開後に再利用しない。
