# Issue #83 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: native CLIのshell worker run、tool-minted trace anchor、worker evidence、canonical review、verification-policy proof derivation/reconciliation。
- 主要な結論: 外部runtime向けの単一lineage constructorをnative CLIにも流用する設計は、型数を抑える局所利益の代わりにsynthetic `RuntimeNodeReport`とverifier `ExecutionTrace`の捏造をintegratorへ要求する局所最適だった。normal CLI recordからだけproofを導出する専用adapterとcanonical review execution recordに分離して解消した。
- 高確度候補数: 2件（いずれも実装時に介入済み）。
- 証拠上の制約: capability retirementはsource-boundary administrationであり、同一storeの通常mutationでは禁止される。そのためE2EはCLI由来のreplayに対する次のauthoritative snapshot状態を用い、store上のretirement workflow自体は観測していない。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- 通常のCaseGraphen操作だけで、producer/verifierのactor、capability、disposition、shared subjectをopaque proofへ到達可能にする。
- proof derivation/reconciliationがreview seamを越えず、evidence acceptanceを代替しない。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | verification-policy module | CLI→store→retained files→review→policyのE2E |
| `M` 評価指標 | constructorの型整合性 | operational reachability、authority provenance、捏造不要性 |
| `N` 変更可能範囲 | generic runtime constructor | native CLI adapter、review record、tests、Skill/docs |
| `T` 時間軸 | proof発行時 | review後、reopen後、capability invalidation後のproof再利用 |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/verification_policy.rs` | generic/native derivationとcurrent-state再検査 | private fieldの実行時使用はtestで観測 |
| 実行 | `tests/verification_lineage_e2e.rs` | real CLI/store positive/negative path | local shell workerのみ |
| 進化 | Issue #81、#83、既存audit | opaque proof導入後に残ったreachability gap | Git共変更の定量分析なし |
| 意味・組織 | `skills/casegraphen-integrate/SKILL.md`、guide | runtime integratorとreviewerの責務境界 | 実組織の運用メトリクスなし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | native runをgeneric runtime reportへ変換 | constructorが一つ | integratorが存在しないtopology/report関係を捏造 | CLI→policy E2E | 10 | C3 | externalization（解消） |
| 2 | reviewにもexecution traceを要求 | producer/verifier validationを対称化 | human reviewに偽traceが必要 | review authority boundary | 9 | C3 | externalization（解消） |
| 3 | generic/nativeの専用adapter分離 | public APIが二系統 | validation branch増加 | 将来3つ以上のruntime family | 3 | C2 | harmless-locality |

## 5. 上位候補の詳細

### Candidate 1: native worker reportのsynthetic runtime変換

#### 識別

- Candidate ID: I83-C1
- 対象実装: 従来の`LedgerLineageDerivation`単一路線。
- 所有モジュール: `verification_policy`、native CLI run、runtime integration。
- 調査者: Codex issue #83 implementation。

#### 事実・推論・仮説

- [Evidence] native runは`WorkerReport`と`ExecutionTrace`を生成し、worker evidenceと`execution_trace_anchor`を同じrun pathでappendする。
- [Evidence] generic constructorは`RuntimeNodeReport`のtopology/node/attempt bindingを要求する。
- [Evidence] `tests/verification_lineage_e2e.rs`は実CLI生成bytesからnative producer proofを導出する。最初のfailed attemptと明示的`--retry-step`によるsuccessful attemptの両方をretainedし、片方のreport/streamsと他方のtraceを混ぜたcross-attemptを拒否する。identifier編集だけをattempt isolationの証拠にはしていない。
- [Evidence] reviewed current revisionで成立したproducer/verifier proofをpre-review replayへ適用すると、review authorityがそのrevisionに存在しないためpolicy reconciliationがfail closedする。一方、current review revisionではproducer subjectをrun base revisionのまま安全に合成する。
- [Evidence] public reconciliation scopeは`subject_kind`/`subject_content_hash`を持ち、native trace hashを`topology_content_hash`と誤表示しない。generic runtimeだけがoptional topology hashを保持する。
- [Inference] generic constructorの再利用ではnative CLIに存在しないtopology metadataをcallerが補う必要があり、opaque proofの入口がcaller-authored adapterへ外部化される。
- [Hypothesis] native worker以外のfirst-party runtimeが追加された場合、現在の二系統で十分かは未検証。

#### 局所的合理性

- 局所目的: proof constructorとbinding shapeを一つに保つ。
- 局所指標: API数、validation function数。
- 直接の受益者: verification-policy module owner。
- 現在得られていた利益: external runtimeのtopology bindingは厳密。
- 導入時の制約: #81ではまずcaller declarationをopaque proofへ置換することが主目的だった。
- 現在も有効な制約: external runtimeの`RuntimeNodeReport` contractは維持する必要がある。
- 失効した制約: native CLIも同じreport型を生成するという暗黙仮定。

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| generic constructorのみ | native `WorkerReport`が入力不能 | synthetic runtime report/topology metadata | host/integrator | native proof導出ごと | Issue #83、型差 |
| claim/topology固定 | worker evidenceにtopology hashがない | metadata捏造またはproof断念 | runtime operator | 全native run | E2E着手時に再現 |

#### 四観測面の証拠

- 構造: `WorkerReport`と`RuntimeNodeReport`は別schema・別責務。
- 実行: 専用adapter後、real CLI E2Eがpositive pathを通過。
- 進化: #81 unit fixtureから#83 operational fixtureへの境界拡張でgapが顕在化。
- 意味・組織: integratorはreport translationではなくuntrusted runtime ingestを所有すべきで、authority factの創作を所有すべきでない。

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 一つのconstructor | native入力不能 | 専用constructor | API追加 | 現在案 |
| モジュール | binding共有 | 分岐なし | private binding共有 | provenance分岐 | 僅差 |
| 機能 | external runtimeは強い | native workflow到達不能 | 両方到達可能 | tests追加 | 代替案 |
| システム | 型数が少ない | caller fabrication | ledger/filesからderive | adapter維持 | 代替案 |
| 運用・組織 | core ownerが単純 | integratorへtrust burden | 責務がsource別に明確 | docs必要 | 代替案 |
| ライフサイクル | 初期実装が速い | 変換が固定化 | source固有証拠を保存 | API compatibility | 代替案 |

- 反転する最小境界: native CLI→policyの機能境界。
- 反転する指標: API数からauthority reachabilityへ変更した時。
- 反転する時間軸: real workflow integration時。

#### 反実仮想

- A 現状維持: synthetic report変換を各callerへ要求。実装量は小さいがauthority ambiguityが残る。
- B 最小改善: native claimへtopology-like metadataを追加。既存語彙を誤用し、migrationと説明コストが残る。
- C 構造変更（採用）: exact native filesとledgerを入力にする専用constructorを追加し、opaque binding/reconcilerを共有。移行の谷はAPI/test追加、rollbackは新API削除で可能。

#### スコアと判定

- `E=3 A=2 F=2 K=2 T=1`、Severity 10、Confidence C3。
- 分類: `externalization`（実装で解消）。

### Candidate 2: verifierにsynthetic execution traceを要求

#### 識別

- Candidate ID: I83-C2
- 対象実装: generic verifier proofのreport/trace対称性。
- 所有モジュール: verification policy、canonical review。

#### 事実・推論・仮説

- [Evidence] normal `review accept|reject`はcanonical review morphismとoperation gateをappendするがworker execution traceを生成しない。
- [Evidence] native verifier constructorはlatest canonical review、exact target、gate capability、distinct actorをledgerからderiveする。
- [Evidence] reopen後に保持済みproofがpolicyを満たさず、再accept後の新reviewだけがproofを再発行できるE2Eを実行した。
- [Inference] human judgmentにworker traceを要求すると、形式対称性のためだけに虚偽artifactを作る誘因になる。
- [Hypothesis] model-based verifierを別workerとして走らせる場合はgeneric verifier constructorが適切であり、native review adapterへ統合すべきではない。

#### 局所的合理性

- 局所目的: producer/verifierのvalidationを同じ関数で扱う。
- 局所指標: code reuse、field symmetry。
- 直接の受益者: core implementer。
- 現在得られていた利益: external verifier reportのexact bytes binding。
- 導入時の制約: verifierもruntime nodeであるユースケースを先に想定。
- 現在も有効な制約: runtime verifierにはreport/traceが必要。
- 失効した制約: canonical reviewもruntime executionであるという仮定。

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| verifier trace必須 | normal reviewからproof不能 | fake trace/report | reviewer integration | reviewごと | Issue #83 criterion |
| subject revisionをverifier traceから読む | later reviewがproducer subjectとずれる | callerがrevisionを合わせる | policy caller | proof joinごと | #81/#83 shared subject rule |

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 共通validation | review入力がない | review専用derive | 関数追加 | 現在案 |
| モジュール | 対称型 | 意味が異なる | canonical reviewをrecordとして使用 | branch | 代替案 |
| 機能 | runtime verifierに強い | human review不能 | 両経路を保持 | docs | 代替案 |
| システム | artifact形式統一 | fabricated provenance | truthful authority chain | API二系統 | 代替案 |
| 運用・組織 | core単純 | reviewer/integrator負担 | review seam明確 | education | 代替案 |
| ライフサイクル | 初期コスト低 | fake artifact慣行 | sourceに忠実 | maintenance | 代替案 |

- 反転する最小境界: review operationとのモジュール境界。
- 反転する指標: field symmetryからprovenance truthfulnessへ変更した時。
- 反転する時間軸: first real CLI proof時。

#### 反実仮想

- A 現状維持: review用fake traceを生成。authority vocabularyが観測より強くなる。
- B 最小改善: review morphismをRuntimeNodeReportへ変換。fake report問題は残る。
- C 構造変更（採用）: canonical review morphismを専用content-bound review execution recordとしてderiveし、producer subjectをopaque producer proofから継承。runtime verifier経路は既存generic APIに残す。

#### スコアと判定

- `E=3 A=2 F=2 K=1 T=1`、Severity 9、Confidence C3。
- 分類: `externalization`（実装で解消）。

## 6. 横断的な補償構造

- 複数候補に共通する変換: source固有recordをgeneric runtime recordへ変換する圧力。
- 複数候補に共通する例外分岐: generic/native provenanceのcurrent-state validation。
- 複数候補に共通する再試行・手動運用: reopen後は新しいreview proofをderiveする必要がある。
- 所有権・KPIに起因する再発構造: core API数の少なさを優先するとintegratorへauthority創作が外部化される。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| generic/native constructorの分離 | API重複 | input schemaとauthority sourceが本質的に異なり、private proof bindingとreconcilerは共有 | bounded contextsの意図的分離 |
| reconciliation時のcurrent-state再検査 | 同じgate/reviewを再検査 | retained proofのreopen/capability invalidationを閉じるため必須 | lifecycle authority |
| review後もproducer subjectがrun base | revision不一致に見える | later reviewはauthority revisionでありsubject置換ではない | content/revision semantics |

## 8. 未検証事項

- model-based verifierをgeneric runtime report経由で実CLI reviewと組み合わせるE2E。
- source-boundary capability retirementを実運用のimport/supersede workflow全体で行う証拠。
- native lineage APIを外部hostから反復利用した際のoperational metrics。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | capability replacement pilot | retained proof invalidationのsource-boundary E2E | replacement/import ADRに従うpilot |
| 2 | external verifier E2E | generic/native API分離の十分性 | runtime report→review seam fixture |
| 3 | API consumer feedback | 二系統APIの運用コスト | pilot integrator review |

## 10. 介入判断の前提

- 変更可能な範囲: experimental verification-policy API、native CLI integration tests、Skills/docs。
- 許容できる移行期間: v0 experimental期間内。
- 一時的に悪化してよい指標: public API数、test runtime。
- 互換性・SLO制約: generic external runtime lineageを弱めない。reconciliationはread-only。
- ロールバック要件: new native constructorsは既存generic constructorsと独立して削除可能。
