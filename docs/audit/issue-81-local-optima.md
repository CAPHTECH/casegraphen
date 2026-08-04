# 実装局所最適監査レポート — Issue #81

## 1. エグゼクティブサマリー

- 調査範囲: `verification_policy` のproducer/verifier lineage、experimental schema、design/integrate Skills
- 主要な結論: caller-declared lineageをledger authorityとして数える問題に加え、revision/subjectの同一視、historical review acceptの再利用、proof発行時だけのcurrent-state検査、tool-minted trace authority typeのcaller forge余地という五つの局所最適を確認した。いずれも単一constructorやgeneric morphism surfaceでは簡潔だが、proof lifecycleとauthority writer境界まで広げると優位性が反転する。最終実装は宣言とopaque proofを分離し、verifier subjectをopaque producer proofから継承し、review traceをsource/target/entryへ結合し、proof利用時にもcurrent CaseSpaceからgate/capability/claim/authority/latest dispositionを再検証する。`custom:execution_trace_anchor`はgeneric CLIとpublic store appendの双方から予約され、crate-privateなcanonical run appendだけがproducer subjectの根を永続化できる。
- 高確度候補数: 5
- 証拠上の制約: 実運用のfailure/latency履歴は未取得。判定は静的構造、negative tests、schema/Skill境界に基づく。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- verification policyが、runtime自己申告ではなく、exact ledger revisionとcontent-bound authorityからactor/capability/disposition/quorumを判定する。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | policy reconcilerへの一回の入力 | runtime ingest、ledger、dispatch/review、schema、agent Skill |
| `M` 評価指標 | policy計算の簡潔さ | authority provenance、差し替え耐性、監査可能性 |
| `N` 変更可能範囲 | lineage input型のみ | constructor、canonical gate、morphism join、schema/Skillを同時変更 |
| `T` 時間軸 | 一回のreconcile | revision進行、retry、将来のstable promotion |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/verification_policy.rs`, `src/native_review.rs` | public field、gate、proof constructor | 静的証拠 |
| 実行 | verification-policy/native CLI unit tests、experimental schema gate | 合成、発行前後reopen拒否、trace-anchor forge拒否、差し替えnegative path | production traceではない |
| 進化 | Issue #81 acceptance criteria、既存anchor hardening | caller-constructible anchorと同型の再発 | Git共変更の定量分析なし |
| 意味・組織 | experimental README、design/integrate Skills | declaredとledger-derivedの語彙境界 | team KPI情報なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | caller-constructible lineageを`LedgerVerifiable`として数える | APIが小さくfixture作成が容易 | runtime callerがactor/capability/quorumを自己申告可能 | runtime→ledger境界 | 12 | C2 | externalization |
| 2 | current authorityまたは各verifier traceをsubject authorityにする | proof単体でrevisionを自己完結できる | producerとlater verifierが合成不能、またはproducerと無関係なsubjectを選択可能 | producer→verifier authority boundary | 13 | C3 | mixed |
| 3 | historical canonical review acceptをlatest dispositionなしで再利用 | historical producer authorityと同じ検索則をreviewにも使える | reopen後も旧acceptがquorumを満たす | evidence review lifecycle | 13 | C3 | time-delayed |
| 4 | proof生成時だけcurrent ledgerを検査 | reconcileがpureでproofを自己完結値として扱える | 発行後のreopen/capability/claim変更を保持済みproofが無視 | proof issuance→use lifecycle | 14 | C3 | time-delayed |
| 5 | CLIまたはpublic store appendがtrace authority typeをmintできる | custom morphism/write APIが一様で拡張容易 | library callerがtool executionのdurable proof recordをforge可能 | caller surface→durable store authority | 13 | C3 | externalization |

## 5. 上位候補の詳細

### 5.1 識別

- Candidate ID: `ISSUE-81-C1`
- 名称: caller-constructible lineage authority
- 対象実装: 旧`ProducerLineage` / `VerifierRecord`とpolicy reconciler
- 所有モジュール / サービス: `verification_policy`
- 所有チーム: CaseGraphen core
- 導入時期: Graph Engineering Plane v0
- 調査者: Codex

### 5.2 事実・推論・仮説

#### 観測された事実

- [Evidence] 旧lineage型はpublic fieldだけを持ち、ledger/store引数なしで作成できた。
- [Evidence] actor/capability比較とquorum計数はその値を`LedgerVerifiable` findingへ使用していた。
- [Evidence] 新constructor testsはcross-revision/claim/attempt、substituted report、retired capability、forged actor、wrong review targetを拒否する。

#### 推論

- [Inference] runtime adapterの正直さへauthorityを外部化すると、policy結果の名称と観測強度が一致しない。

#### 未検証仮説

- [Hypothesis] 実runtime pilotで、ledger-derived proof生成時の追加I/Oとhash計算が有意な負荷になる可能性がある。

### 5.3 局所的合理性

- 局所目的: policyアルゴリズムとquorum semanticsを独立して実装・テストする。
- 局所指標: input型とconstructor数、fixture作成量。
- 直接の受益者: core実装者とunit-test作者。
- 現在得られていた利益: pure functionとして簡単に照合できる。
- 導入時の制約: experimental v0で、acceptance seamへ直結しなかった。
- 現在も有効な制約: runtime declarationの保存と診断は必要。
- 失効した制約: operational hostと実runtime integrationの追加により、ledger join不能を許容する理由はなくなった。

### 5.4 評価条件

- `B`: reconcilerからruntime→ledger→review lifecycleへ拡張
- `M`: 実装簡潔性からprovenance完整性へ拡張
- `N`: local type renameからschema/Skillを含む境界変更へ拡張
- `T`: 単一実行から複数revision・retryへ拡張

### 5.5 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| public lineageをledger fact扱い | adapterがactor/capabilityの正しさを暗黙保証 | review seamでpolicy結果を再解釈 | operator/reviewer | verificationごと | Skillsの「runtime verifierはuntrusted」規則 |
| revision bindingなし | 古いidentity/quorumの再利用 | callerがrevision整合を手作業確認 | host integrator | revision進行ごと | cross-revision negative test |

### 5.6 四観測面の証拠

#### 構造

- 宣言型はSerde可能なinput record、opaque proofはprivate fieldかつDeserializeなしに分離した。
- proof constructorは`check_operation_gate`を呼び、capability authority ruleを複製しない。

#### 実行

- unit testsはexact bytesのpositive derivationと差し替えnegative casesを実行する。
- experimental schema gateは宣言契約のRust/JSON round-tripを対象にする。

#### 進化

- anchorで実施済みの「declared vs tool-observed」分離をlineageにも適用し、trust語彙を揃えた。

#### 意味・組織

- runtime integratorは宣言を保存できるが、review/acceptance operatorへ強い証明として渡せない。

### 5.7 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 小さいpure API | provenanceなし | constructorが増える | fixtureが複雑 | 旧案 |
| モジュール | unit test容易 | trust tier混同 | 型で誤用防止 | join code増加 | 新案 |
| 機能 | quorum計算可能 | forged quorum可能 | exact binding | ledger material必要 | 新案 |
| システム | adapter自由度 | authority外部化 | canonical rule一元化 | hash/I/O追加 | 新案 |
| 運用・組織 | integration容易 | reviewerが再検証負担 | audit vocabulary明確 | operator setup | 新案 |
| ライフサイクル | 初期実装が速い | stable化後の移行ロックイン | v0中にtrust境界固定 | breaking change | 新案 |

- 反転する最小境界: module/runtime trust boundary
- 反転する指標: API簡潔性からauthority provenanceへ
- 反転する時間軸: 複数revisionでの再利用時

### 5.8 反実仮想

#### A. 現状維持

- 定常コスト: callerの正直さをreviewerが別途確認。
- 将来コスト: stable schemaで強い名称が固定される。
- リスク: forged actor/capability/quorum。

#### B. 最小限の局所改善

- 変更: 旧型を`Declared*`へ改名するだけ。
- 利益: 語彙上の過大claimを削減。
- 残る問題: strong verification pathが存在せずpolicyを運用利用できない。
- 移行コスト: 小。

#### C. 境界をまたぐ構造変更

- 変更: declared schemaとopaque ledger-derived proof constructorを分離。
- 成立条件: exact case revision、canonical gate/capability、dispatch/review morphism、report/trace bytesが存在。
- 定常利益: authority provenanceと差し替え耐性。
- 新たなコスト: proof生成fixtureとcontent hashing。
- 移行の谷: callerは旧public structをstrong reconcilerへ直接渡せない。
- ロールバック: experimental v0の間はdeclared-only診断へ戻せるが、strong claimは失われる。

### 5.9 スコア

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 2
- `K` KPI乖離: 3
- `T` 時間ロックイン: 2
- `Severity`: 12
- `Confidence`: C2

### 5.10 判定

- 分類: `externalization`
- 判定理由: local pure APIの簡潔性のためauthority検証をruntime caller/reviewerへ押し出し、runtime境界で優位性が反転する。
- 反証となり得る情報: policy結果が今後も完全にdiagnosticで、actor/capability/quorumを一切の運用判断に使わないという製品決定。
- 未検証事項: large quorumでのproof derivation cost。
- 次に取得すべき証拠: real runtime pilotでproof derivation latencyとfailure distributionを記録。

### 5.11 Candidate `ISSUE-81-C2`: observed/trace revisionをcanonical subject authorityにする

#### 識別

- Candidate ID: `ISSUE-81-C2`
- 名称: current-authorityまたはcaller trace revisionのsubject authority化
- 対象実装: 中間`derive_lineage_binding`のauthority-entry検索、`LedgerLineageBinding.case_revision_id`、producerなしのverifier constructor
- 所有モジュール / サービス: `verification_policy`
- 所有チーム: CaseGraphen core
- 導入時期: Issue #81の初期opaque-proof実装
- 調査者: Codex + 最終invariant監査

#### 事実・推論・仮説

##### 観測された事実

- [Evidence] 初期実装はauthority entryの`target_revision_id`が`case_space.revision.revision_id`と一致することを要求し、そのcurrent revisionをproducer/verifier間で一致すべき`case_revision_id`として保持した。
- [Evidence] append-only ledgerではproducer dispatch authorityがrevision Pを生成した後、verifier review authorityは後続revision Vを生成する。PとVが同一current revisionであることはない。
- [Evidence] 中間修正は各proofの`case_revision_id`をcallerが渡す`ExecutionTrace.base_revision_id`から独立に導出した。trace hashはmorphismへ結合されても、verifier constructorはproducer proofを要求せず、review時系列のbase revisionとdeployment subject revisionを同じfieldへ載せた。
- [Evidence] 最終修正ではverifier constructorがopaque `LedgerDerivedProducerProof`を必須とし、verifierのcanonical subject revisionをproducer proofから継承する。verifier traceの`base_revision_id`、`result_revision_id`、`appended_entry_ids`はそれぞれreview morphismのsource、target、entryへ結合され、subject authorityとしては使われない。
- [Evidence] `producer_and_later_verifier_proofs_compose_on_the_shared_subject_revision`は、current ledgerからhistorical producer authorityとlater verifier authorityを導出し、異なるobserved/recording revisionで同じproducer-derived subjectへ合成してpolicyを満たす。

##### 推論

- [Inference] 「current entryだけを認める」局所安全策も「各content-bound traceのbaseをsubjectとする」局所修正もproof単体では説明しやすい。しかし前者は正規のappend-only workflowを不可能にし、後者はproducerが定めたdeployment subjectとverifier reviewの時系列revisionを混同する。

##### 未検証仮説

- [Hypothesis] 長いmorphism historyでhistorical authorityを線形探索すると、proof derivation latencyが増える可能性がある。

#### 局所的合理性

- 局所目的: 古いauthority entryを単純に拒否し、各proofを一つのtraceだけで自己完結させる。
- 局所指標: constructor一件の鮮度、引数の少なさ、比較条件の単純さ。
- 直接の受益者: proof constructor実装者と単体test。
- 現在得られていた利益: authorityがcurrent ledger headにあること、またはtrace baseとmetadataが一致することを一proof内で確認できた。
- 導入時の制約: producer/verifier proofを独立fixtureで検証し、時系列合成testがまだなかった。
- 現在も有効な制約: proofはcurrent canonical ledgerから導出され、authority entryとbytesが完全一致しなければならない。
- 失効した制約: authority event自体がcurrentでなければならないという制約。append-only history内のcanonical eventもcurrent ledgerの構成要素である。

#### 評価条件

- `B`: proof constructor一件からproducer→later verifier lifecycleへ拡張
- `M`: head/trace自己整合から正規workflowのcomposabilityとproducer-owned subject authorityへ拡張
- `N`: equality条件一箇所からconstructor signature、binding model、trace/morphism join、testを同時変更
- `T`: 同一revisionから複数append revisionへ拡張

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| authority target=currentを要求 | producer proofはreview append後に導出不能 | producer proofを事前保存する | runtime integrator | verification runごと | current equalityとappend-only revision sequence |
| proof subject=currentとする | producer Pとverifier Vのbinding mismatch | verifierを同一revisionへ捏造するかstrong reconcileを使わない | reviewer/operator | quorum memberごと | `same_binding`のrevision比較 |
| verifier trace baseをsubjectにする | callerがproducerと無関係なsubjectをreview traceへ選べる | reconcilerでproducerと事後比較 | security reviewer | verifier proofごと | producer引数なしの中間constructor |
| historical producer entryを無条件に拒否 | canonical historyがauthorityとして再利用不能 | caller-declared lineageへ退行 | security reviewer | long-running runごと | 合成test追加前のAPI制約 |

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | current equality/trace baseだけで判定 | 時刻概念とsubject authorityを一つに圧縮 | observed/subject/recording revisionを分離 | 比較条件増加 | 初期・中間案 |
| モジュール | proofが自己完結 | producer不在でverifier subjectを選べる | opaque producer proofからsubject継承 | constructor結合増加 | 修正案 |
| 機能 | proof単体は新鮮 | producer+verifier合成不能または別subject | later reviewをproducer subjectへ結合 | producer proof必須 | 修正案 |
| システム | head/trace-only mental model | append-only ledgerとauthority ownershipに矛盾 | event timeとdeployment authorityが明確 | 三つのrevision関係を検査 | 修正案 |
| 運用・組織 | operator説明が短い | 正規review seamがpolicy利用を壊す | 通常のdispatch→review順序を維持 | retained trace必要 | 修正案 |
| ライフサイクル | short-lived testに適合 | 長時間runほど導出不能 | historical canonical proofが継続利用可能 | index最適化候補 | 修正案 |

- 反転する最小境界: producerとverifierの二morphismを含む機能境界
- 反転する指標: constructor単体のhead freshnessからverification lifecycleのcomposabilityへ
- 反転する時間軸: 最初のreview morphism append時点

#### 反実仮想

##### A. 現状維持

- 定常コスト: producer proofをreview前に保持しなければならず、それでもsubject revisionがlater verifierと一致しない。
- 将来コスト: quorumが増えるほど異なるtarget revisionが増え、strong pathが恒常的に不成立。
- リスク: caller-declared pathへの退行、またはrevision checkの場当たり的無効化。

##### B. 最小限の局所改善

- 変更: authority entryの`target_revision_id == current`条件を削除し、各trace baseをsubjectにする。
- 利益: historical producer entryを検索でき、proof単体のmetadataは自己整合する。
- 残る問題: verifier trace baseはreview morphism source revisionでありdeployment subjectではない。producer proofなしではverifierがcanonical subjectを継承できない。
- 移行コスト: 小さいがauthority replay攻撃を防ぐ意味モデルが不足。

##### C. 境界をまたぐ構造変更

- 変更: `observed_case_revision_id`を分離し、producer traceから一度だけsubjectを導出する。verifierはopaque producer proofを必須としsubjectを継承し、verifier trace base/result/entryはreview morphism source/target/entryの時系列証明として検査する。
- 成立条件: producer traceがcase space、operation gate、topology/node/attempt、report/trace bytesへcontent-boundであり、verifier review morphismとtraceがcanonical history上で一致する。
- 定常利益: stale/fabricated authorityを拒否しつつ、dispatch→later review→quorumの正規時系列を合成可能。
- 新たなコスト: observed、subject、review source/targetの語彙が増え、producer proof依存、history探索、追加invariant testが必要。
- 移行の谷: metadataの`case_revision_id`をauthority targetではなくsubject revisionとして生成し直す必要がある。
- ロールバック: observed/subject fieldsを保持したままhistorical lookupをfeature-gateできる。単一revisionモデルへ戻すとcomposabilityを失う。

#### スコア

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 3
- `K` KPI乖離: 3
- `T` 時間ロックイン: 2
- `Severity`: 13
- `Confidence`: C3

#### 修正後判定

- 分類: `mixed`（`time-delayed` + `externalization`）
- 判定理由: current equalityは一つ後のreview appendで合成を壊し、各verifier trace baseをsubject authorityにする中間案はdeployment subjectの選択をcaller側へ外部化する。両方を合成testとconstructor signatureの監査で確認した。
- 修正後状態: 解消。proofは導出時に観測したcurrent revisionを保持し、verifierはopaque producer proofからcontent-bound deployment subjectを継承する。verifier traceはreview morphismのsource/target/entryへ結合され、subject authorityにはならない。historical authorityはcaller snapshotではなくcurrent canonical ledgerからのみ導出する。
- 反証となり得る情報: producerとverifierが常に同じmorphismで原子的に記録される別ledger model。ただし現在のreview seam/append-only modelとは一致しない。
- 未検証事項: large historyでの探索性能と、複数retryをまたぐsubject revision選択。
- 次に取得すべき証拠: long-running pilotでobserved/subject revision差、history depth、proof derivation latencyを記録する。

### 5.12 Candidate `ISSUE-81-C3`: reopen後のhistorical accept再利用

#### 識別

- Candidate ID: `ISSUE-81-C3`
- 名称: historical review authority without latest disposition
- 対象実装: historical authority entryを許可した中間`derive_ledger_verifier_proof`
- 所有モジュール / サービス: `verification_policy`、`native_eval`のevidence review derivation
- 所有チーム: CaseGraphen core
- 導入時期: historical producer authority対応時
- 調査者: Codex + 最終invariant監査

#### 事実・推論・仮説

##### 観測された事実

- [Evidence] producerとlater verifierを合成するにはhistorical producer dispatch entryをcurrent canonical ledgerから検索する必要がある。
- [Evidence] 同じ検索許容をaccepted reviewへそのまま適用すると、その後にcanonical `reopen`がappendされても、古いaccept morphism IDを指定してverifier proofを再生成できる。
- [Evidence] `latest_evidence_review_entries`はevidence targetごとの最新log entryを選ぶ、native evaluator/evidence status系と共有されるcanonical selectorである。
- [Evidence] 修正後constructorは指定review morphismがこのlatest entryと完全一致しなければ`verifier_review_not_current`で拒否する。
- [Evidence] 合成testの後半はreopenをappendし、同じhistorical acceptからのproof再導出が失敗することを確認する。

##### 推論

- [Inference] historical authorityは一律に安全または危険なのではない。dispatchという過去の発生事実は保持できるが、review dispositionは後続reopen/reject/supersedeで現在効力が変わるため、同じ検索policyを共有できない。

##### 未検証仮説

- [Hypothesis] evidence以外の将来のreview targetにもreopen相当の効力変更が導入された場合、target-kindごとのcanonical latest selectorが必要になる。

#### 局所的合理性

- 局所目的: producer/verifierの両authorityを同じhistorical morphism lookupで扱い、constructorを対称にする。
- 局所指標: lookup実装の共通性、過去entryのcontent binding。
- 直接の受益者: verification module実装者。
- 現在得られていた利益: accepted review entryがlogに残る限りproofを再構築できる。
- 導入時の制約: historical producer対応を優先し、review効力の時間変化をconstructor testへ含めていなかった。
- 現在も有効な制約: review morphismのcontent hash、actor、gate、trace source/target/entryはcanonical historyから検証する。
- 失効した制約: 「一度canonical acceptなら将来もverifier authority」という仮定。reopenは明示的にその効力を取り消す。

#### 評価条件 `B/M/N/T`

- `B`: 一つのaccepted review entryからevidence review lifecycle全体へ拡張
- `M`: historical content integrityからcurrent disposition correctnessへ拡張
- `N`: verification module内lookupからnative evaluatorのcanonical latest selector再利用へ拡張
- `T`: accept記録時点からreopen/reject/supersede後へ拡張

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| accepted historical reviewを存在だけで採用 | reopen後もquorum memberとして数えられる | reconciler利用者が最新review statusを別途確認 | policy caller | review変更ごと | reopen negative test |
| producer/reviewへ同一history policyを適用 | event factとrevocable dispositionを混同 | target kindごとの例外分岐 | core maintainer | authority kind追加ごと | dispatchとreviewのlifecycle差 |
| verification側でlatest ruleを再実装 | evaluatorと将来drift | 二箇所同時変更 | reviewer/evaluator owner | review semantics変更ごと | native evaluatorが既にselectorを所有 |

#### 四観測面の証拠

##### 構造

- append-only logにはacceptとreopenが両方残り、morphism ID検索だけでは現在効力を決められない。
- 修正は`native_eval::latest_evidence_review_entries`をcrate-internalに公開し、verification側でlatest判定ruleを複製していない。

##### 実行

- positive halfはacceptがlatestならproducer＋verifier proofが合成可能であることを確認する。
- negative halfはreopen append後に同じaccept morphismを指定し、`verifier_review_not_current`を観測する。

##### 進化

- review actionが増減してもlatest entry selectionはevaluatorの一つのdecision ruleへ追随する。verification module独自のaccept-cacheを持たない。

##### 意味・組織

- dispatch historyの「発生した事実」とreview historyの「現在有効なdisposition」を同じauthority概念へ押し込まない。evidence semanticsの所有者はnative evaluatorに維持される。

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | morphism ID lookupだけでproof化 | latest statusを読まない | canonical latest比較 | map deriv出コスト | 初期案 |
| モジュール | producer/reviewer lookupが対称 | revocable authorityを誤分類 | authority kindごとのlifecycle保持 | 非対称な検証 | 修正案 |
| 機能 | accept proofをいつでも再構築 | reopenがquorumへ反映されない | review seamの取消効力を維持 | current ledger必須 | 修正案 |
| システム | append-only historyを活用 | acceptance ledgerの現在状態と矛盾 | evaluatorとverificationが同一selector | crate-internal依存 | 修正案 |
| 運用・組織 | callerのproof cacheが長寿命 | operatorのreopenが無視される | operator dispositionが即時反映 | proof再導出必要 | 修正案 |
| ライフサイクル | accept直後は正しい | 最初のreopenで優位性反転 | accept/reopenを通じfail closed | target-kind拡張検討 | 修正案 |

- 反転する最小境界: 同一evidenceへのaccept→reopenという二entry lifecycle
- 反転する指標: historical proof再構築性からcurrent review authority correctnessへ
- 反転する時間軸: reopen entry append直後

#### 反実仮想

##### A. 現状維持

- 定常コスト: callerがproof利用前に別APIでlatest reviewを確認し続ける。
- 将来コスト: check/use raceとrule drift。quorum cacheの無効化手順が必要。
- リスク: reviewerが明示的にreopenしたclaimをaccepted verifier authorityとして再利用する。

##### B. 最小限の局所改善

- 変更: verification module内でmorphism logの最後の同target entryを独自探索する。
- 利益: reopen後のhistorical acceptを拒否できる。
- 残る問題: native evaluatorのcanonical evidence status ruleと重複し、target kind、malformed review、将来actionでdriftする。
- 移行コスト: 小さいがauthority rule duplicationを導入する。

##### C. 境界をまたぐ構造変更

- 変更: native evaluatorが所有する`latest_evidence_review_entries` selectorを再利用し、proof対象morphismがexact latest entryであることをconstructor内で要求する。
- 成立条件: current canonical case spaceをconstructorへ渡し、review targetがevidenceである。
- 定常利益: reopen/reject後のhistorical acceptをfail closedにし、evidence statusとverification proofが同じdecision ruleを読む。
- 新たなコスト: verification moduleからnative evaluatorへのcrate-internal dependencyとlatest map導出。
- 移行の谷: 既存のhistorical verifier proofはcurrent reviewが同じacceptでない限り再導出不能になる。
- ロールバック: selector checkだけを戻せるが、reopen authority holeが再発するため安全な運用rollbackではない。

#### スコア

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 3
- `K` KPI乖離: 3
- `T` 時間ロックイン: 2
- `Severity`: 13
- `Confidence`: C3

#### 実装後判定

- 分類: `time-delayed`
- 判定理由: accept直後は正しいhistorical lookupが、reopen append後には明示的なreview取消を無視することを実行testで確認した。
- 修正後状態: 解消。verifier proofはopaque producer proofを必須とするだけでなく、exact latest canonical evidence review entryからのみ生成できる。reopen後の旧acceptはcontentが改変されていなくてもauthorityを失う。
- 反証となり得る情報: reviewが永久・取消不能という別ドメイン契約。現在の`ReviewAction::Reopen`と矛盾する。
- 未検証事項: evidence以外のreview targetにおけるcurrent-disposition semantics。
- 次に取得すべき証拠: accept→reopen→再accept、reject→reopen、supersedeを含むreview lifecycle matrix。

### 5.13 Candidate `ISSUE-81-C4`: proof発行時だけのcurrent-state検査

#### 識別

- Candidate ID: `ISSUE-81-C4`
- 名称: issue-time-only lineage authority
- 対象実装: 中間`reconcile_verification_policy`と保持済みopaque proof
- 所有モジュール / サービス: `verification_policy`
- 所有チーム: CaseGraphen core
- 導入時期: opaque proof constructor導入時
- 調査者: Codex + 最終invariant監査

#### 事実・推論・仮説

##### 観測された事実

- [Evidence] opaque proofは発行時にcurrent CaseSpace、gate/capability、claim、authority morphism、latest reviewを検査するが、保持後もRust値として再利用できる。
- [Evidence] 中間reconcilerはCaseSpaceを受け取らず、proof内の発行時bindingだけでactor/capability/quorumを計算した。
- [Evidence] review accept後にproofを発行し、reopenをappendした後でも、その保持済みverifier proofのfieldは変化しない。
- [Evidence] 最終reconcilerはcurrent `CaseSpace`を必須にし、case validity/identity、canonical operation gateと現在のcapability、claim/topology、authority morphism、log-derived latest review dispositionを再検証する。
- [Evidence] reopen後の既発行proofを渡すnegative pathは`verifier_review_no_longer_effective`を出し、`policy_satisfied=false`、quorum不成立にする。

##### 推論

- [Inference] opaque/non-Deserializeはproofのforgeを防ぐが、proofが表すauthorityの失効までは防がない。proofのcontent integrityとcurrent authorization validityは異なる時間軸である。

##### 未検証仮説

- [Hypothesis] large quorumでproofごとにcurrent gate/historyを再検証すると、同一capability/claimの重複走査が性能候補になる。

#### 局所的合理性

- 局所目的: reconcileをCaseSpace非依存のpure functionに保ち、発行済みproofを安価に集約する。
- 局所指標: reconcile引数数、I/O/ledger走査回数、proof再利用性。
- 直接の受益者: policy callerとquorum aggregator。
- 現在得られていた利益: proof発行後はledgerを再読せずpolicyを再計算できる。
- 導入時の制約: proof fieldsをprivateにするforge対策が中心で、revocation/use-time semanticsが未定義だった。
- 現在も有効な制約: deterministic core calculationはmutationせず、authority ruleはcanonical modulesを再利用する。
- 失効した制約: 発行時に正しかったauthorityが利用時にも正しいという仮定。review reopenとcapability lifecycleが反証する。

#### 評価条件 `B/M/N/T`

- `B`: proof constructorからissuance→retention→reconcile lifecycleへ拡張
- `M`: proof forge耐性からcurrent authorization correctnessへ拡張
- `N`: pure reconcileだけからcurrent CaseSpace/gate/evaluatorを含む再検証へ拡張
- `T`: proof発行時点から任意の後続revisionへ拡張

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| proofをtimeless valueとして集約 | reopen後もaccept dispositionが残る | callerが利用直前にreview status確認 | quorum aggregator | reconcileごと | issued-proof reopen negative |
| reconcileがCaseSpace非依存 | capability retire/claim変更を観測不能 | proof cache TTLや手動invalidaton | operator | revision変更ごと | current gate/claim checks追加 |
| proofごとに発行時revisionを保持 | 現在authorityとの距離が不明 | observed revision比較だけでstaleness推定 | integrator | long-running run | exact current semanticsはrevision equalityだけでは不足 |

#### 四観測面の証拠

##### 構造

- 中間APIは`policy, producer, verifiers, anchors`だけを受け、current ledgerへのjoinがなかった。
- 最終APIは先頭に`&CaseSpace`を受け、`current_lineage_findings`が既存`check_operation_gate`、claim、authority、latest review statusを再利用する。

##### 実行

- 同一proofはreopen前にpolicyを満たし、reopen後のcurrent CaseSpaceではfail closedになる。proof bytes/fields自体は変更していない。

##### 進化

- capability lifecycleやreview actionが増えても、利用時にcanonical gate/evaluatorを再実行するため、proof cache独自の失効ruleを増やさない。

##### 意味・組織

- proof発行者とproof利用者が別process/時刻でも、利用者がcurrent ledgerを提示しなければauthority claimを成立させない。revocation負担をoperatorの手順からcoreへ戻す。

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | pure reconcile | current stateを読まない | current findings追加 | 引数/分岐増加 | 初期案 |
| モジュール | proofだけで集約 | revocation ruleを所有不能 | canonical gate/eval再利用 | module依存増加 | 修正案 |
| 機能 | proof cacheが高速 | reopenがquorumへ反映されない | current dispositionを保証 | ledger replay必要 | 修正案 |
| システム | producer/reviewerとaggregatorを疎結合 | stale authorityを外部化 | use-time authorization | current store availability | 修正案 |
| 運用・組織 | offline集約可能 | operatorがinvalidatonを負担 | revoke/reopenが即反映 | offline成立不可 | 修正案 |
| ライフサイクル | 発行直後は正しい | 最初のauthority変更で反転 | 全revisionでfail closed | repeated validation | 修正案 |

- 反転する最小境界: proof発行後に一つのreopen/capability変更がappendされる境界
- 反転する指標: reconcile purity/再利用性からcurrent authority correctnessへ
- 反転する時間軸: proof発行後の最初のauthority-affecting revision

#### 反実仮想

##### A. 現状維持

- 定常コスト: callerがproof cache invalidation、latest review、capability statusを同期管理する。
- 将来コスト: authority ruleごとのTTL/notification/manual recoveryが増える。
- リスク: reopen済みaccept、retired capability、変更済みclaimを古いproofで再利用する。

##### B. 最小限の局所改善

- 変更: proofの`observed_case_revision_id`とcaller-declared current revisionを比較する。
- 利益: revisionが進んだstale proofを一律拒否できる。
- 残る問題: unrelated revisionでも全proofを無効化し、historical producer authorityの正当な利用を再び壊す。caller-declared revisionの正典性も不足。
- 移行コスト: 小さいがC2を再発させる。

##### C. 境界をまたぐ構造変更

- 変更: reconcileへcurrent canonical CaseSpaceを必須化し、各proofのgate/capability、claim/topology、authority morphism、verifier dispositionをuse timeに再検証する。
- 成立条件: current CaseSpaceをreplay可能で、proofがcanonical bindingをprivateに保持する。
- 定常利益: unrelated revisionを許容しつつ、authorityに影響する変更だけをfail closedにする。
- 新たなコスト: reconcile時のledger evaluation/history lookupとcurrent store依存。
- 移行の谷: proof-only/offline callerはcurrent CaseSpace取得を追加する必要がある。
- ロールバック: pure calculationは内部`reconcile_bound_verification_policy`としてtest可能だが、public authority pathからcurrent checkを外してはならない。

#### スコア

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 3
- `K` KPI乖離: 3
- `T` 時間ロックイン: 3
- `Severity`: 14
- `Confidence`: C3

#### 実装後判定

- 分類: `time-delayed`
- 判定理由: 発行時に正しいproofがreopen後もfield上は正しいまま残り、中間reconcilerではauthority変更を観測できない反転を実testで確認した。
- 修正後状態: 解消。public reconcileはcurrent CaseSpaceなしに呼べず、保持済みproofを現在のcanonical gate/capability/claim/authority/latest dispositionへ再joinする。mutationは行わない。
- 反証となり得る情報: capability/review/claimが永久不変という契約。現在のlifecycle/reopen semanticsと矛盾する。
- 未検証事項: quorum内の重複current validationコストとproof batch最適化。
- 次に取得すべき証拠: capability retire、claim supersede、authority morphism corruptionを発行後に行うuse-time matrix。

### 5.14 Candidate `ISSUE-81-C5`: trusted trace authority typeのgeneric forge余地

#### 識別

- Candidate ID: `ISSUE-81-C5`
- 名称: caller-authored `custom:execution_trace_anchor`
- 対象実装: generic `morphism propose/apply` validation、`NativeCaseStore::append_morphism`、canonical run append、producer proof authority lookup
- 所有モジュール / サービス: native CLI morphism operations、native store、worker execution、verification policy
- 所有チーム: CaseGraphen core
- 導入時期: `execution_trace_anchor`をcustom morphism typeとして導入した時点
- 調査者: Codex + 最終invariant監査

#### 事実・推論・仮説

##### 観測された事実

- [Evidence] producer proofはaccepted `custom:execution_trace_anchor` morphismを、CaseGraphen tool pathが実traceを観測・記録したauthorityとして読む。
- [Evidence] `CaseMorphismType::Custom(String)`自体はgeneric proposal surfaceからcallerが指定できるため、type名を予約しなければcallerが同じauthority labelを作れる。
- [Evidence] CLIだけの予約では不十分だった。`NativeCaseStore::append_morphism`はpublic APIであり、library callerはgeneric CLI validationを迂回してcaller-built `MorphismLogEntry`を永続化できた。
- [Evidence] 最終実装ではpublic `append_morphism`が`custom:execution_trace_anchor`をstore mutation前に拒否し、crate-private `append_execution_trace_anchor`だけが型を再確認してcanonical run pathからのappendを許す。
- [Evidence] canonical run pathはtrace anchorだけを専用store methodへ送り、その他のmorphismはpublic appendへ送る。
- [Evidence] generic morphism validatorも`review`、`evidence_attach`、`custom:execution_trace_anchor`をtool-minted予約型として拒否し、早いrefusalを維持する。
- [Evidence] `public_store_append_cannot_mint_an_execution_trace_anchor`はpublic library APIからのappendが拒否され、historyが増えないことを確認する。`generic_morphism_cannot_forge_an_execution_trace_anchor`はCLI surfaceでも同じforgeを拒否する。
- [Evidence] producer subjectはgeneric morphism metadataではなく、tool-minted anchored execution traceのbase revisionからのみ導出される。

##### 推論

- [Inference] content hashやcanonical log membershipだけでは「誰がこのauthority recordを書けたか」を証明しない。writer exclusivityは最深のdurable mutation境界で強制されて初めてauthority typeの意味になる。CLI validationはdefense-in-depthであり、authority boundaryそのものではない。

##### 未検証仮説

- [Hypothesis] 将来tool-minted custom morphism typeが増えた場合、文字列denylistだけでは登録漏れが再発する可能性がある。

#### 局所的合理性

- 局所目的: custom morphism extensionをgeneric CLIとpublic store APIから一様に提案・appendできるようにする。
- 局所指標: morphism type dispatchの単純さ、新type追加時の変更箇所数、CLI surface上のforge拒否。
- 直接の受益者: extension author、CLI integrator、library integrator。
- 現在得られていた利益: core enum変更なしで新しいrecord typeを追加できる。
- 導入時の制約: custom typeは主に意味ラベルで、tool execution proofとして読まれる予約namespaceが未定義だった。
- 現在も有効な制約: authorityを持たないcustom typesはcaller proposalとして利用できる。
- 失効した制約: すべてのcustom typeが同じtrust tierという仮定。trace anchorはproducer proofのrootとして強く読まれる。

#### 評価条件 `B/M/N/T`

- `B`: generic CLI parserからpublic library/store append→worker execution→producer proof authorityへ拡張
- `M`: extension flexibilityからwriter provenance/authority integrityへ拡張
- `N`: custom type一律許可からstore-owned capability method、CLI defense-in-depth、両surfaceのnegative testへ拡張
- `T`: proposal/append時からdurable historyと後日のproof再構築まで拡張

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| custom typeをcaller自由入力 | tool execution proof labelをforge可能 | verification側でmetadata全fieldを再確認 | security reviewer | producer proofごと | trace anchor authority lookup |
| CLIだけでwriter exclusivityを検査 | public library callerがCLIを迂回してdurable logへappend | 各library callerが同じ予約規則を手動実装 | integrator/operator | caller追加・auditごと | public `NativeCaseStore::append_morphism` |
| 予約型追加を各consumer任せ | consumer追加時にdeny漏れ | 複数validatorへ文字列追加 | core maintainer | tool-minted type追加ごと | review/evidence/traceの共通性 |

#### 四観測面の証拠

##### 構造

- `Custom(String)`はauthority/non-authorityを型で区別しないため、public store appendが予約型を拒否し、crate-private capability methodだけがcanonical run pathへmint権限を与える。
- generic CLI validatorも同じ予約型を拒否するが、これは早期診断とdefense-in-depthであり、永続authorityはstoreが所有する。
- producer constructorはanchor morphismとtrace/hashを検証し、store boundaryで保証されたwriter exclusivityと組み合わせてproofを導出する。

##### 実行

- CLI forge negative testはgeneric proposal/apply validatorで拒否する。
- store regression testはpublic APIから同じentryをappendして拒否されることと、拒否後もhistory lengthが変化しないことを確認する。

##### 進化

- worker executionがmintする型をstore-owned capabilityへ移し、将来追加されるlibrary callerも自動的に保護する。CLI側の予約は既存custom extensionを壊さず早期拒否を提供する。一方、将来のtool-minted型を専用store capabilityへ登録し忘れるリスクは残る。

##### 意味・組織

- runtime toolが得る「実行を観測してdurable recordをmintする」authorityと、callerが得る「morphismを提案・通常appendする」権限を分離する。永続recordのauthority ownerをstoreとcanonical run pathへ戻す。

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | custom文字列を一様処理 | authority区別なし | reserved check | 分岐追加 | 初期案 |
| モジュール | extension APIが小さい | public store APIでCLIを迂回 | store boundaryで拒否 | 専用capability method | 修正案 |
| 機能 | callerが新typeを即利用 | producer proof rootを作れる | anchored traceだけを採用 | tool path必須 | 修正案 |
| システム | logはappend-only | public callerがauthority recordを永続化 | durable writer boundaryとcontent bindingを両立 | cross-module契約 | 修正案 |
| 運用・組織 | extension authorの自律性 | security reviewerが生成経路を追跡 | errorで境界を即時通知 | reserved docs必要 | 修正案 |
| ライフサイクル | type追加が高速 | authority consumer追加後に危険化 | tool-minted型を予約 | registry候補 | 修正案 |

- 反転する最小境界: CLI validationを迂回できるpublic store APIと、そのrecordをproducer proof authorityとして読むconsumerの境界
- 反転する指標: extension変更量からwriter provenanceへ
- 反転する時間軸: trace-anchor consumer導入時点

#### 反実仮想

##### A. 現状維持

- 定常コスト: producer proof constructorがcaller-created anchorを完全に識別する追加署名/attestationを必要とする。
- 将来コスト: tool-minted custom typeごとにconsumer側forge検査が増える。
- リスク: CLIを通らないlibrary callerがtool executionを装うauthority morphismをdurable historyへappendする。

##### B. 最小限の局所改善

- 変更: verification constructorで特定metadata flagやactor IDを確認する。
- 利益: 既知fixtureのforgeを拒否できる。
- 残る問題: flag/actorもgeneric morphism callerが記述でき、authority ruleがconsumerへ重複する。
- 移行コスト: 小さいがwriter exclusivityを成立させない。

##### C. 境界をまたぐ構造変更

- 変更: public store appendがtool execution proofとして読むmorphism typeを拒否し、crate-privateな`append_execution_trace_anchor`だけをcanonical run pathへ公開する。generic CLIでも同じ型を早期拒否し、producer subjectはそのanchored traceから導出する。
- 成立条件: すべてのdurable morphism writeが`NativeCaseStore`を通り、canonical run pathだけがcrate-private capability methodへ到達できる。
- 定常利益: CLI以外の現在・将来のlibrary callerにもwriter authorityが保証され、CLI validationとproducer-side content validationがdefense-in-depthになる。
- 新たなコスト: reserved type inventoryとextension authorへの明示的 refusal。
- 移行の谷: caller-authored同名custom morphismは拒否され、非authority名へ移行が必要。
- ロールバック: 予約を外すとauthority forge余地が即時再発するため、安全なrollbackではない。

#### スコア

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 3
- `K` KPI乖離: 3
- `T` 時間ロックイン: 2
- `Severity`: 13
- `Confidence`: C3

#### 実装後判定

- 分類: `externalization`
- 判定理由: generic extensionの局所的単純さが、tool execution provenanceの検証をconsumer/operatorへ押し出す。forge negative testとproducer authority consumerの二面で反転を確認した。
- 修正後状態: 解消。generic CLIだけでなくpublic `NativeCaseStore::append_morphism`からも`custom:execution_trace_anchor`をmintできない。canonical run pathだけがcrate-private専用appendを通じて永続化でき、tool-minted anchored traceだけがproducer proofのsubject revisionを供給する。CLI検査はstore authorityに対するdefense-in-depthとして維持した。
- 反証となり得る情報: generic morphism path自体がtool署名済みでcaller編集不能という別境界。現在のproposal fileモデルとは一致しない。
- 未検証事項: reserved custom typeの登録漏れを自動検出するinventory。
- 次に取得すべき証拠: 新規tool-minted custom type追加時にgeneric forge negative fixtureを要求するconformance test。

## 6. 横断的な補償構造

- 複数候補に共通する変換: runtime report IDをledger identityとして読み替える変換。
- 複数候補に共通する例外分岐: なし。
- 複数候補に共通する再試行・手動運用: revision一致の手確認。
- 所有権・KPIに起因する再発構造: 「最新revisionだけを安全」とする局所KPIが、append-only historyを使うend-to-end verification完了率を悪化させる。
- その他の再発構造: adapterの「report completeness」とledgerの「authority acceptance」を同じ成功指標で扱うと再発する。
- authority-kindに起因する再発構造: immutable event factとrevocable dispositionを同じhistorical lookup policyへ統一すると再発する。
- 時間軸に起因する再発構造: proof issuance時の正しさをuse-time authorityへ昇格すると、reopen/revoke/claim変更で再発する。
- writer境界に起因する再発構造: consumerが強く読むcustom typeをCLIだけで予約し、public durable appendをgeneric writerへ開くと、content validationが正しくても再発する。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| runtime attestationをproofから除外 | strong proofがsession freshnessを証明しない | CaseGraphenから観測不能であり、強く扱う方が誤り | `NotObservableHere`を維持する意図的境界 |
| canonical gateへの結合 | module間結合増加 | authority ruleの一元化とcapability revocation反映に必要 | セキュリティ・監査要件による合理的結合 |

## 8. 未検証事項

- verifier proofを使う実runtime pilotの運用証拠。
- morphism logが大規模な場合のauthority entry探索コスト。
- retry/supersede後も同じsubject revisionを使う条件と、別subjectへ切り替える条件。
- accept→reopen→再acceptを含むreview lifecycle全体でlatest selectorとproof derivationが一致するか。
- proof発行後のcapability retire、claim supersede、authority corruptionをuse-time reconcileが拒否するか。
- tool-minted custom morphism typesの予約inventoryが網羅的か。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | real producer/verifier pilot | constructor入力が実運用で十分か | retained report/traceとreview morphismで統合test |
| 2 | proof derivation benchmark | hash・log探索コスト | 1k/100k morphism fixture benchmark |
| 3 | review lifecycle matrix | reopen以外のdisposition変更 | accept/reject/reopen/supersede sequence test |
| 4 | retained-proof revocation matrix | use-time再検証の網羅性 | capability/claim/review変更後に同じproofを再利用 |
| 5 | reserved authority type inventory | 将来のgeneric forge／store capability登録漏れ | tool-minted type、public store refusal、専用append capability、CLI validatorのconformance gate |

## 10. 介入判断の前提

- 変更可能なチーム・サービス範囲: experimental core/schema/Skills。
- 許容できる移行期間: v0 stable promotion前。
- 一時的に悪化してよい指標: fixture作成量とconstructor call数。
- 互換性・規制・SLO制約: acceptance authority ruleを複製しない。
- ロールバック要件: declared inputの保存・診断経路は維持する。
