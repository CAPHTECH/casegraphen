# Issue #68 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査モード: deep-dive / intervention
- 調査範囲: verification anchor declaration、tool-observed proof、CaseGraphen artifact/trace provenance、policy result vocabulary
- 主要な結論: hash形式と値の一致だけをpolicy reconciler内で検査する方式は、関数内では決定論的だが観測責任をcallerへ外部化する局所最適だった。raw declarationとopaque proofを型・結果名・constructorで分離し、強い結果はexact bytesとledger/adapter provenanceを要求するよう修正した。
- 高確度候補数: 1（実装で解消）
- 証拠上の制約: 実storeで生成されたpositive trace fixtureは未取得。positive pathはdeterministic trusted-reference adapter、trace pathはforgery/substitution refusalで検証した。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- runtime self-reportがhashを複写してもtool-observed anchorへ昇格しないこと。
- artifact/traceのidentity、bytes、ledger provenanceが一体で検証されること。
- verification policy resultがnormal evidence acceptanceを代替しないこと。

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | anchor enumの値比較 | caller → artifact/trace observation → policy → evidence review seam |
| `M` 評価指標 | deterministic equality | observation strength、forgery耐性、語彙の誤用防止 |
| `N` 変更可能範囲 | reconciler関数 | public input/result typesとCaseGraphen trace/artifact join |
| `T` 時間軸 | 1回のpolicy評価 | adapter追加、runtime self-report再利用、stable化後のAPI利用 |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/verification_policy.rs:88-451,560-739` | declaration/proof/provenance/result API | 静的証拠 |
| 実行 | `cargo test verification_policy --lib` | 10 tests | positive CaseGraphen trace storeなし |
| 進化 | `git log -- src/verification_policy.rs` | Graph Engineering Plane導入コミット以降 | 履歴が短い |
| 意味・組織 | Issue #68、既存module doc、Issue #51 audit | evidence acceptanceとのauthority分離 | team運用データなし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | caller-constructible `AnchorObservation` | 小さく汎用的なvalue API | 観測責任と意味解釈をcaller/consumerへ外部化 | trust boundary | 11 | C2 | externalization（解消済み） |

## 5. 上位候補の詳細

### 5.1 識別

- Candidate ID: I68-C1
- 名称: declaration-as-observation
- 対象実装: 旧`AnchorObservation`と`anchors_satisfied`集約
- 所有モジュール: `verification_policy`

### 5.2 事実・推論・仮説

#### 観測された事実

- [Evidence] raw値は`DeclaredAnchorObservation`だけがDeserializeでき、結果は`declared_anchors_match`である（`src/verification_policy.rs:88-125,410-451`）。
- [Evidence] `ToolObservedAnchorProof`の全fieldとprovenance enumはprivateである（`src/verification_policy.rs:128-205`）。
- [Evidence] CaseGraphen artifact constructorはcanonical evaluation、content-addressed ID、cell type/lifecycle、metadata hash、bytesをjoinする（`src/verification_policy.rs:223-278`）。
- [Evidence] trace constructorはaccepted execution-trace anchor、trace/result revision、entry ID、worker/stdout/stderr bytesをjoinする（`src/verification_policy.rs:280-354`）。
- [Evidence] copied hashes、artifact substitution、unanchored/substituted trace testsがfail closedを確認する（`src/verification_policy.rs:916-990`）。

#### 推論

- [Inference] callerがhashを2回渡して成立するものは整合するdeclarationであり、world observationではない。両者を同じ型に置くとconsumerが強さを取り違える。

#### 未検証仮説

- [Hypothesis] host adapterが実store traceからproofを作る際のI/Oコストは、既存trace anchor verificationと統合すれば重複を避けられる。

### 5.3 局所的合理性

- 局所目的: source hashとtest exitを同じ小さなenumで扱う。
- 局所指標: API/実装量、unit test容易性。
- 直接の受益者: verification policy caller。
- 現在得られていた利益: transport-independentな決定論的照合。
- 導入時の制約: experimental v0、evidence acceptanceへ直結しない。
- 現在も有効な制約: policy resultは受理権限を持たない。
- 失効した制約: caller declarationしか観測源がないという制約。

### 5.4 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| declarationとobservationを同型化 | result consumerが強度を推測 | docsで「authorityではない」と注意 | adapter/Skill/利用者 | policy利用ごと | 旧module docとIssue #68 |
| hashだけを受け取る | bytes/provenance検証がcaller規律になる | 外部anchor adapterの暗黙検査 | runtime integrator | anchorごと | copied-hash test |

### 5.5 四観測面の証拠

- 構造: raw declaration型はstrong reconcilerの引数型にならず、proof fieldはprivate。
- 実行: deterministic reference bytesは成功し、identityを保ったbytes substitutionは拒否。
- 進化: v0段階で型分離することで、stable後の名称/APIロックインを回避。
- 意味・組織: CaseGraphen ledgerか明示的なin-crate adapterが観測を所有し、runtime callerはdeclarationだけを所有。

### 5.6 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | enum値比較が簡潔 | provenanceなし | constructor増加 | 実装量増 | 旧案 |
| モジュール | 単一API | vocabularyが過強 | weak/strong API明示 | 型数増 | 新案 |
| 機能 | fixture作成容易 | self-report forgery | exact bytes join | fixture整備 | 新案 |
| システム | adapter自由度 | trust owner不明 | ledger/adapter owner明示 | integration必要 | 新案 |
| 運用・組織 | callerが自己完結 | consumerが監査負担 | proof発行責任を限定 | adapter所有が必要 | 新案 |
| ライフサイクル | 初期APIが小さい | stable後に誤用固定 | v0で強度を型へ固定 | migration | 新案 |

- 反転する最小境界: verification policy moduleのpublic trust boundary
- 反転する指標: 実装量から観測強度・誤用耐性へ広げた時
- 反転する時間軸: 2つ目のruntime adapter/consumerが導入された時

### 5.7 反実仮想

#### A. 現状維持

- 定常コスト: docsによる注意とcaller監査。
- リスク: copied hashを`anchors_satisfied`と解釈する。

#### B. 最小限の局所改善

- 変更: `anchors_satisfied`を`declared_anchors_match`へrename。
- 利益: 過強な語彙を修正。
- 残る問題: 将来tool observationを同じ型へ再混入しやすい。

#### C. 境界をまたぐ構造変更（採用）

- 変更: declaration、opaque proof、CaseGraphen artifact/trace constructor、trusted adapter capabilityを分離。
- 成立条件: CaseGraphen case evaluationとcontent hashingを再利用できること。
- 定常利益: raw inputからstrong resultへの型経路がない。
- 新たなコスト: constructor/fixture数とtrace bytes I/O。
- 移行の谷: experimental callerが旧enumからweak/strongのどちらかを選ぶ。
- ロールバック: v0では可能だがtrust vocabularyを再び弱める。

### 5.8 スコアと判定

- `E`: 3
- `A`: 2
- `F`: 2
- `K`: 2
- `T`: 2
- `Severity`: 11/15
- `Confidence`: C2
- 分類: `externalization`、実装で解消
- 反証となり得る情報: 全consumerがraw declarationを弱い値として扱う形式検証。ただし公開型とfield名はそれを強制していなかった。

## 6. 横断的な補償構造

- 変換: declarationからproofへの変換は存在しない。
- 例外分岐: invalid/missing/substituted bytesはtyped `PolicyFinding`。
- 再試行・手動運用: proofはexact bytesとcontent addressへ結合され、再観測も決定論的。
- 所有権: ledger observer / trusted adapter / runtime declarer / evidence reviewerを分離。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| policyに`policy_satisfied`を残す | 名前が強く見える | strong resultはopaque proofのみ受け取り、型docがevidence acceptanceではないと明記 | policy内部の集約として妥当 |
| independent minds/fresh contextを常にfalse | conservativeに見える | CaseGraphen単独では観測不能という境界を正しく保存 | intentional refusal |
| traceで4ファイルを再hash | I/O重複候補 | trace自身がそれらのhashをcommitしており、substitution検出に必要 | integrity cost |

## 8. 未検証事項

- 実CaseGraphen storeが生成したpositive execution-trace proofのend-to-end test。
- OS/TEE等の外部attestation adapter。
- proof observation I/Oの大規模benchmark。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | real store trace fixture | positive trace pathの統合性 | `run --step` fixture storeから4ファイルを観測 |
| 2 | adapter integration test | capability配布境界 | MCP/host reference adapterでproof発行 |
| 3 | I/O profile | trace再hashコスト | 1MB/100MB outputs benchmark |

## 10. 介入判断の前提

- 変更可能範囲: experimental verification policy API。
- 許容移行期間: v0中の破壊的型変更。
- 一時的悪化: caller fixtureとconstructorが増える。
- 互換性・規制制約: stable evidence/review authorityを変更しない。
- ロールバック要件: raw declarationをstrong reconcilerへ戻さない。
