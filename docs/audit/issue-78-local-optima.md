# 実装局所最適監査レポート — Issue 78

## 1. エグゼクティブサマリー

- 調査範囲: reviewed topology/policy authority、graph compiler、operational MCP host、bundle persistence、control-plane inventory、resource reservationへの引渡し。
- 主要な結論: proposal compileだけを公開する旧hostは、acceptance authorityを推測しない小さなreference boundaryとして局所的に合理的だった。しかしreview済みdeploymentを実行する機能境界では、opaque authority導出をcustom Rust hostとoperatorへ外部化し、公式経路を分断する局所最適だった。専用toolがstore replayからauthorityを導出する構造へ置換した。さらに、manifestとcaller hashを受けるauthority constructorはadapter規約に従う限り小さかったが、library境界では未検証bytesをproofへ昇格できる局所最適だったため、全bundle検証を所有するopaque `VerifiedDeploymentBundle`を追加した。
- 高確度候補数: 2（いずれも修正済み）。
- 証拠上の制約: 静的構造とlocal process E2E fixtureは確認したが、独立MCP clientによるreview→compile→remote runtime、production latency、crash注入は未計測。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- reviewされたexact topology/policyだけを、callerがreview modeやauthority hashを構築せずoperational hostからcompileできる。
- generated planとruntime outputはreview済みtopology authorityから区別し、引き続き`unreviewed` / `accepted:false`で停止する。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | proposal compiler関数 / reference host | topology review → store replay → reviewed compile → persisted bundle → resource/runtime handoff |
| `M` 評価指標 | hostの小ささ、fail-closed、mutation非所有 | authority完全性、公式workflow完結性、改竄検出、consumer実装量 |
| `N` 変更可能範囲 | compiler libraryかhostの一方 | compiler、store adapter、control plane、schema/catalog、docs/Skills、E2E |
| `T` 時間軸 | 一回のproposal生成 | review済みdeploymentの反復運用とstable promotion後のcontract進化 |

制約はRust 1.80、experimental v0、append-only ledger、MCP bearer authenticationとCaseGraphen authorityの分離、hostがacceptance-ledger mutationを所有しないことである。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/graph_compiler.rs:205-434,437-645`、`src/bin/casegraphen-mcp-host.rs:218-261,749-787` | opaque verified bundle/binding、reviewed compile、bundle verification | 静的証拠 |
| 実行 | `tests/resource_host_e2e.rs:24-212,268-389` | happy path、wrong claim/policy、stale revision、tampered artifact | synthetic local process |
| 進化 | Issue #78、ADR 0020/0023、旧proposal-only product surface | library-only authorityからoperational toolへの変更増幅 | PR/運用履歴なし |
| 意味・組織 | `docs/execution-topology-review.md`、`docs/guides/mcp-operational-host.md`、integrate/operate Skills | reviewer、host、compiler、runtimeのauthority境界 | 実組織KPIなし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | operational hostをproposal compileだけに限定 | hostがledger authorityを扱わず小さくfail-closed | custom Rust caller、手動authority bridge、非公式workflow | host単体 → review-to-runtime機能 | 9 | C2 | `externalization`、修正済み |
| 2 | authority constructorがmanifestとcaller hashを直接受ける | persistence adapterを選ばず小さいAPI | 全artifact/inventory検証をcaller規約へ押し出し、未検証bytesを強いproofへ渡せる | trusted adapter → public library API | 8 | C2 | `externalization`、修正済み |

## 5. 上位候補の詳細 — LO-78-1

### 5.1 識別

- Candidate ID: LO-78-1
- 名称: reviewed compilation authorityのlibrary-only局所化
- 対象実装: 旧`compile_deployment_bundle` operational pathと`reviewed_compilation_mode` library API
- 所有モジュール / サービス: `graph_compiler`、`casegraphen-mcp-host`、native store
- 所有チーム: CaseGraphen maintainers / host integrator
- 導入時期: Graph Engineering Plane v0からIssue #78まで
- 調査者: Codex host-authority review lane

### 5.2 事実・推論・仮説

#### 観測された事実

- [Evidence] canonical coreには、accepted `ExecutionTopology` reviewだけからopaque `CompilationMode::Reviewed`を生成する`reviewed_compilation_mode`が存在する（`src/graph_compiler.rs:336-434`）。
- [Evidence] 旧operational hostはproposal modeを固定し、review済みcompileをtool inventoryへ公開していなかった（Issue #78の対象とADR 0020の旧surface）。
- [Evidence] 修正後hostは`case_space_id`と`claim_cell_id`を受け、current revisionをstore replayで照合してからopaque modeを導出する。caller supplied mode/review/hashは入力型にない（`src/bin/casegraphen-mcp-host.rs:218-261`）。
- [Evidence] compilerはtopology hash、case space、accepted revision、policy manifest hashを再検査し、生成planを`unreviewed`に保つ（`src/graph_compiler.rs:437-568,577-645`）。
- [Evidence] `verify_deployment_bundle`はmanifest content address、typed manifest一致、重複のないexact artifact inventory、10個の必須artifact、全artifact bytes/hash/length、topology ID/case space/content hashを一度に検査し、field非公開の`VerifiedDeploymentBundle`だけを生成する（`src/graph_compiler.rs:215-350`）。
- [Evidence] `reviewed_deployment_authority`はcaller hashとmanifestを直接受けず、`VerifiedDeploymentBundle`だけを受ける（同`:353-426`）。host loaderもfilesを収集するだけでcanonical verifierへ委譲する（`src/bin/casegraphen-mcp-host.rs:749-787`）。
- [Evidence] wrong claim、policy substitution、stale revisionはE2Eで拒否される（`tests/resource_host_e2e.rs:268-341`）。

#### 推論

- [Inference] proposal-only hostはauthority誤昇格を避けたが、reviewed deploymentの正規利用者にstore replayとopaque mode生成を再実装させ、decision boundaryを製品外へ押し出していた。
- [Inference] separate toolで入力をidentityとcurrent revisionに限定すると、proposal inspectionとreviewed deploymentを混同せず、canonical ruleをcompiler/storeに一意に保てる。

#### 未検証仮説

- [Hypothesis] 多数artifact / 多数同時compile時にもfull bundle hash検証と同期書込みのp95/p99が許容範囲に収まる。
- [Hypothesis] review acceptance直後以外のledger進行を含む実運用で、exact accepted revision workflowがoperator retryを過度に増やさない。

### 5.3 局所的合理性

- 局所目的: operational hostがCaseGraphen authorityを誤って生成・受理しないようproposal-onlyにする。
- 局所指標: tool数、store依存、authority-bearing branch数、誤受理可能性。
- 直接の受益者: reference host maintainerと初期security reviewer。
- 現在得られていた利益: compiler outputとruntime outputは常にunreviewedで、hostはledger mutationを所有しなかった。
- 導入時の制約: MCPはreference adapterであり、canonical store replay delegateが未実装だった。
- 現在も有効な制約: bearer tokenはCaseGraphen authorityではない。generated plan/runtime resultは別review seamを通る。
- 失効した制約: operational hostがcanonical storeとcompilerへ接続できず、reviewed modeを安全に導出できないという制約。

### 5.4 評価条件

- `B`: proposal compilerからreview→compile→resource handoffへ拡張。
- `M`: fail-closed branch数からauthority完全性と公式workflow完結性へ拡張。
- `N`: host単独でなくstore、compiler、schemas、consumer docsを同時変更。
- `T`: 一回のinspectionから複数runtime/clientが使う運用期間へ拡張。

### 5.5 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| hostはproposal modeのみ | accepted topologyからofficial deploymentを作れない | custom Rust callerがstore replayとmode導出 | host integrator | reviewed runごと | Issue #78、旧product surface |
| compileをreviewより前に置く | runtime bundleがreview authorityを持たない | 後段topology reviewをauthority bindingとして手動追跡 | operator/reviewer | deploymentごと | ADR 0020旧workflow |
| library型だけがopaque proofを持つ | MCP clientはauthority provenanceを観測不能 | adapter固有response/manifestを追加 | adapter owner | host実装ごと | compiler/host旧境界 |

### 5.6 四観測面の証拠

#### 構造

- 修正後はcontrol-plane enum/catalogからhost delegate、canonical compilerまで一つのtyped pathを持つ（`src/control_plane.rs:33-92`）。
- `ReviewedCompilerInput`はmode、review ID、topology hashを受けず、authority decisionをcallerへ複製しない。
- persisted manifestはreview claim、review ID/revision、topology/policy hashes、artifact inventoryを保持する。
- persistence adapterとauthority constructorの間にはopaque `VerifiedDeploymentBundle`があり、必須inventoryやtopology identityのdecision ruleをhostへ複製しない。

#### 実行

- E2Eはattach → topology-review accept → reviewed compile → reserve → reconcile → `needs_review`を実processで通す（`tests/resource_host_e2e.rs:24-145,447-617`）。
- wrong claim、policy substitution、stale revision、artifact tamperがstable refusalでfail closedする（同`:268-389`）。
- compiler unit testsは正しいbundleのopaque化、caller digest差替え、manifest/artifact substitutionを検査する（`src/graph_compiler.rs:1361-1386`）。

#### 進化

- core-only reviewed modeとproposal-only hostを別々に進化させた結果、公式surfaceに穴が残った。修正は新toolを追加しつつ、compiler decision rule自体は複製せず委譲した。
- control-plane schema/catalog、product-surface inventory、Skillsが同時更新され、将来のsurface driftをconformance対象へ含める。

#### 意味・組織

- reviewerはtopology/policy contentをacceptし、hostはその事実をreplayし、compilerはdeployment bytesを生成し、runtime outputは再びunreviewedとなる。四者のauthorityが混ざらない。
- custom host integratorが暗黙のauthority ownerになる補償を除去した。

### 5.7 境界拡張と優位性反転

| 評価境界 | 旧proposal-only案の利益 | 旧案のコスト | 代替案（専用reviewed tool）の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | mode固定で単純 | reviewed pathなし | store replay branch | 入力/拒否増加 | 旧案 |
| モジュール | hostがledger型を知らない | compiler機能を公開できない | opaque constructor再利用 | hostがstoreへ依存 | 代替案僅差 |
| 機能 | proposal inspectionは安全 | review済み実行が非公式 | official review→compile | exact revision調整 | 代替案 |
| システム | runtime outputを誤受理しない | custom hostごとauthority drift | 一つのcanonical authority path | schema/catalog拡張 | 代替案 |
| 運用・組織 | reference host ownerの責務が小さい | operator/integratorが手動bridge | 所有権とrefusalが明示 | store運用が必要 | 代替案 |
| ライフサイクル | 初期変更量が少ない | runtime/client追加ごとcustom glue | future adapterが同じtoolを利用 | v0 migration | 代替案 |

- 反転する最小境界: hostモジュールからreview-to-runtime機能へ広げた時点。
- 反転する指標: host実装量からauthority完全性とconsumer実装量。
- 反転する時間軸: 二つ目のoperational consumerがreviewed deploymentを必要とする時点。

### 5.8 反実仮想

#### A. 現状維持（旧proposal-only）

- 定常コスト: reviewed compileはcustom Rust callerのみ。
- 将来コスト: host/runtime familyごとにauthority bridgeが増える。
- リスク: callerがcurrent claim metadataや自己申告hashをauthorityとして扱う。

#### B. 最小限の局所改善

- 変更: MCP requestへcaller supplied `mode: reviewed`とhash群を追加。
- 利益: tool追加が小さく既存compile handlerを再利用できる。
- 残る問題: review存在・revision・policy bindingの判断をcaller入力へ戻し、authority bypassを生む。
- 移行コスト: 小さいがtrust modelを弱めるため採用不能。

#### C. 境界をまたぐ構造変更（採用）

- 変更: separate reviewed tool、store replay、opaque mode、compiler再検証、content-addressed persistence、inventory/docs更新。
- 成立条件: configured native storeとprivate artifact rootが同一hostのdecision boundary内にあること。
- 定常利益: callerがauthorityをmintできず、official workflowが完結する。
- 新たなコスト: store I/O、tool/schema surface、bundle verification、revision refusal。
- 移行の谷: proposal-only clientとreviewed clientの二経路を明示し、v0 schema/exampleを同時更新する。
- ロールバック: reviewed toolを無効化してproposal-onlyへ戻せる。既存review/ledger historyは変更しない。

### 5.9 スコア

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 1
- `K` KPI乖離: 2
- `T` 時間ロックイン: 1
- `Severity`: 9 / 15
- `Confidence`: C2（静的authority pathとlocal E2E/negative evidenceが一致。production実測なし）

### 5.10 判定

- 分類: `externalization`、修正済み。
- 判定理由: reference hostの小ささという局所利益が、reviewed deployment利用者へauthority bridgeを押し出し、機能境界で優位性が反転した。
- 反証となり得る情報: hostが恒久的にlint/inspection専用でruntime deploymentを製品成果に含めないならproposal-onlyは`harmless-locality`である。しかしoperational hostとresource/runtime workflowを公開しているため現状には該当しない。
- 未検証事項: independent client、remote runtime、concurrent compile、large bundle、crash後のpartial directory回復。
- 次に取得すべき証拠: issue-76 pilotをreviewed tool経由で再実行し、retained MCP transcriptとbundle hashesを保存する。

### 5.11 追加候補 — LO-78-2: bundle検証責務をpersistence adapterへ外部化するconstructor

#### 局所的合理性と評価条件

- 局所目的: canonical review bindingとmanifest metadataを比較する小さなauthority constructorを保つ。
- 直接の受益者: graph compiler API利用者とalternate persistence adapter実装者。
- `B`: trusted host adapter内からpublic library callerまで拡張すると優位性が反転する。
- `M`: parameter数/adapter自由度から、proof constructorの観測強度とdecision-rule一意性へ広げる。
- `N`: authority constructorだけでなくbundle byte verificationとhost loaderを同時変更する。
- `T`: 一つのcanonical hostから第三者adapterが追加される期間へ広げる。

#### 四観測面・補償ハロー

- 構造: 旧constructorはmanifestとdigest形式を検査したが、コメントで「persistence adapterが先に全bytesを検証する」ことを要求していた。型はその事前条件を表現しなかった。
- 実行: hash形式だけ正しいcaller値やartifact substitutionの反例をunit testで検査できるようになった。
- 進化: host側に必須artifact loopとtopology parsingを置くと、次adapterで同じruleが再実装される変更増幅が生じる。
- 意味・組織: `ReviewedDeploymentAuthority`という強い名前の前提をコメント/runbookだけでadapter ownerに負担させる。

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 規模 | 証拠 |
|---|---|---|---|---|---|
| constructorはmanifest/hashだけを見る | 未検証artifact集合がauthority候補になる | 各adapterがinventory/hash/topologyを事前検査 | adapter owner/security reviewer | adapterごと | 旧constructor contract |
| host loaderがbundle semanticsを所有 | alternate adapterでrule drift | docsとnegative fixtureを複製 | maintainer | contract変更ごと | 修正前host loader |

#### 優位性反転とA/B/C

| 評価境界 | 旧案の利益 | 旧案のコスト | opaque verified bundleの利益 | 新コスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 少ない型/比較 | hidden precondition | 一つの追加型 | verifier実装量 | 旧案僅差 |
| モジュール | persistence非依存 | proof強度を型で表せない | bytes→proofを一元所有 | compilerがartifact inventoryを知る | 新案 |
| システム/将来adapter | adapter自由 | rule複製・authority bypass | 全adapterが同じconstructor | `DeploymentBundle`組立が必要 | 新案 |

- A: コメント契約を維持する。将来adapterごとにsecurity reviewが必要。
- B: host loaderだけを強化する。canonical hostは安全だがdirect/alternate callerが残る。
- C（採用）: `verify_deployment_bundle`だけがopaque proofを生成し、authority constructorはproofだけを受ける。移行時は全callerのbundle組立が必要だがexperimental v0内で同時変更できる。
- `Severity`: 8 / 15 (`E=2, A=2, F=2, K=1, T=1`)
- `Confidence`: C2（static type boundaryとdigest/substitution unit tests）
- 判定: `externalization`、修正済み。

## 6. 横断的な補償構造

- 複数候補に共通する変換: claim/review logからauthority hash群をadapterが再構築する補償をopaque compiler bindingへ集約した。
- 複数候補に共通する例外分岐: proposal inspectionは残すが、reviewed executionとはtool名・manifest mode・response authorityを分ける。
- 複数候補に共通する再試行・手動運用: stale revisionは自動`current`置換せず、再読取と明示resubmitを要求する。
- 所有権・KPIに起因する再発構造: hostの「何も決めない」を単独KPIにすると、必要なcanonical delegationまでcustom integratorへ外部化する。

## 7. 候補ではなかったもの（false positives）

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| proposal compileを別toolとして残す | 二重compile surface | inspectionとauthority-bearing deploymentは意味が異なり、同じhandlerへmode flagを入れる方がtrust boundaryを曖昧にする | intentional separation |
| compilerがreviewed hashを再検証 | reviewとの重複 | direct library caller、store/artifact corruptionへのdefense-in-depthで、ruleは同じbindingから比較するだけ | security boundary |
| generated planを`unreviewed`にする | topology review済みなのに再review | topology/policy authorityと具体plan/runtime evidenceは別contentであり、自動継承させないことが中心要件 | acceptance boundary |
| `VerifiedDeploymentBundle`が`DeploymentBundle`を包む | wrapper/表現重複 | raw bytes集合と検証済みproofは観測強度が異なり、private fieldがconstructor bypassを防ぐ | opaque proof boundary |

## 8. 未検証事項

- independent MCP clientによるfull reviewed compile transcript。
- large/many artifact bundleのsync I/O、p95/p99/RSS。
- bundle persistence途中のkillと再実行時の完全回復。
- accepted review後にunrelated ledger revisionが進む運用頻度とre-review/compile retry負担。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | issue-76四runtimeのreviewed compile pilot | official workflowのend-to-end成立 | independent MCP clientでtranscript/hash/haltsをretain |
| 2 | crash-injection bundle test | partial persistenceとidempotent recovery | artifact書込各点でkillし同じrequestをreplay |
| 3 | concurrent/large bundle benchmark | filesystem contentionとlatency | fixed artifact sizes/concurrencyでp50/p95/p99計測 |

## 10. 介入判断の前提

- 変更可能なチーム・サービス範囲: experimental compiler、MCP host、schema/catalog、Skills。
- 許容できる移行期間: v0期間中。proposal-only pathはinspection互換として維持。
- 一時的に悪化してよい指標: compile latency、response/manifest bytes、tool count。
- 互換性・規制・SLO制約: Rust 1.80、append-only ledger、bearer認証はauthorityではない、runtime outputはuntrusted。
- ロールバック要件: ledger historyを変更せずreviewed toolだけを無効化可能であること。

## 11. 実装後の判定

反実仮想Cを採用した。hostはcaller modeを受けず、current storeからopaque reviewed bindingを導出する。bundle bytesはcanonical `verify_deployment_bundle`を通って初めてopaque proofになり、authority constructorはcaller hash/manifestを直接受けない。compilerはtopology/policy/reviewを再検証し、outputのacceptanceを昇格させない。調査した境界ではmaterialなauthority局所最適は残っていない。
