# 実装局所最適監査レポート — Issue 79

## 1. エグゼクティブサマリー

- 調査範囲: reviewed deployment bundle、operational resource reservation/release/supersede、atomic allocator journal、runtime resource expectation、current revision assertion、artifact trust boundary。
- 主要な結論: allocatorがcaller supplied topologyだけを純粋に検査する旧境界は、資源競合とatomicityの所有者として局所的に合理的だった。しかし複数clientへ公開するoperational host境界では「何のreview済みdeploymentのための予約か」を認証済みcallerへ外部化し、arbitrary reservation/denial-of-serviceを許す局所最適だった。reviewed bundle authorityとjournal provenanceへ結合した。実装途中に生じたaccepted-review revisionのrelease再利用も、current revision assertionへ修正した。加えて、同じ`AtomicResourceAllocator`がpublicなreviewed/unreviewed mutationを併存させるAPIはlibrary callerの誤用を型で防げなかったため、unreviewed mechanicsを明示的な`UnreviewedResourceJournal`へ隔離した。
- 高確度候補数: 3（いずれも修正済み）。
- 証拠上の制約: local filesystem/process E2Eと静的journal検査を使用した。multi-host負荷、長期journal、OS crash、悪意ある同時clientは未計測。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- resource reservationがcanonicalにreview/compileされたexact topology、policy、bundle、review revision、node、attempt、declarationへ結合される。
- reserve/release/supersedeがcurrent case revisionとatomic allocator stateの両方を検査し、runtime reconciliationが同じjournal provenanceを要求する。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | resource conflict pure function / allocator journal | topology review → bundle → reserve → runtime allocation → reconcile → release/supersede |
| `M` 評価指標 | conflict-free grant、atomic append、idempotency | deployment authority、current revision、provenance完全性、multi-client abuse resistance |
| `N` 変更可能範囲 | `resource_allocator`のみ | compiler authority、host/store、journal schema、runtime bundle、tests/docs |
| `T` 時間軸 | 一回のreservation | ledger進行、attempt lifecycle、journal growth、複数deployment運用 |

制約はresource protocolをacceptance ledgerへ統合しないこと、allocator journalをcanonical active-state ownerにすること、runtime allocationをuntrusted observationとして扱うこと、Rust 1.80とexperimental v0である。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/resource_allocator.rs:35-99,217-505,519-727`、`src/graph_compiler.rs:215-332` | opaque authority projection、journal、release/supersede joins | 静的証拠 |
| 実行 | `tests/resource_host_e2e.rs:24-212,268-389`、`tests/resource_expectation_bundle.rs` | reviewed reserve/reconcile、stale release、tamper、bundle validation | synthetic local process |
| 進化 | Issue #79、ADR 0022、前回host authority review | pure evaluator→atomic allocator→review-bound allocatorの段階的変更 | production historyなし |
| 意味・組織 | MCP host guide、integrate Skill、experimental schemas | host authentication、CaseGraphen authority、runtime assertionの分離 | real operator/tenant KPIなし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | caller topologyを根拠にatomic reserve | allocatorがacceptance domainから独立し再利用可能 | authenticated clientが任意graphでexclusive resourceを占有、reconcile provenanceを手作業 | allocator module → multi-client operational host | 11 | C2 | `mixed` (`externalization` + security boundary)、修正済み |
| 2 | accepted-review revisionをreleaseのbase revisionに再利用 | provenance field一つでreserve/releaseを結合 | ledger進行後にcurrent release不能、stale revisionが成功 | journal event → case/resource lifecycle | 8 | C2 | `externalization`、修正済み |
| 3 | 一つのoperational allocatorにpublic reviewed/unreviewed mutationを併存 | mechanics reuseとtest fixtureが簡単 | library callerがunreviewed grantをoperational reservationとして誤用可能 | private implementation → public API/第三者caller | 7 | C2 | `externalization`、修正済み |

## 5. 上位候補の詳細

### 5.1 LO-79-1: caller topologyを根拠にしたoperational reservation

#### 識別

- 対象実装: 旧host `reserve_resources` → `AtomicResourceAllocator::reserve`
- 所有モジュール: MCP host、resource allocator/protocol、graph compiler
- 直接の受益者: allocator maintainer、pure protocol caller
- コスト負担者: shared resource利用者、operator、runtime reconciler

#### 事実・推論・仮説

- [Evidence] allocatorの競合判定はtopology/declaration/reservationとjournal active stateを使い、atomic append/idempotencyを所有する。
- [Evidence] 旧hostはbearer-authenticated callerのtopologyをそのまま予約評価へ渡し、accepted topology reviewやdeployment bundleを要求しなかった（Issue #79対象）。
- [Evidence] 修正後hostはpersisted bundle全artifactをhash検証し、storeから`ReviewedDeploymentAuthority`を再導出して`reserve_reviewed`へ渡す（`src/bin/casegraphen-mcp-host.rs:313-357,749-834`）。
- [Evidence] journal bindingはclaim、review、topology、policy、bundle、accepted revision、case space、node、attempt、declaration hashを保持する（`src/resource_allocator.rs:35-62,217-295`）。
- [Evidence] 修正後`AtomicResourceAllocator`のpublic mutationは`reserve_reviewed` / `disposition_reviewed`だけで、authorityなしmutationはprivate methodとなる。明示的な`UnreviewedResourceJournal`だけがそれをmechanics評価として公開する（`src/resource_allocator.rs:157-210,224-267,348-367`）。
- [Inference] 旧案のatomicityは「同じclient-supplied universe内の安全性」には有効だが、そのworkがreview authorityを持つかというコストをallocator外へ押し出した。
- [Hypothesis] tenant別quotaやreservation TTLがなくても、review-bound authorityだけで実運用のresource starvationを十分抑止できる。

#### 局所的合理性

- 局所目的: topology resource claimとactive journalの競合をatomicに判定する。
- 局所指標: grant correctness、journal durability、pure resource ruleの再利用性。
- 現在も有効な利益: allocatorはCaseGraphen evidence acceptanceを行わず、resource conflict decisionを一箇所に保持する。
- 導入時の制約: reviewed compileとbundle authorityがoperational hostから利用できなかった。
- 失効した制約: reservation時にcanonical reviewed bundleを取得・検証できないこと。

### 5.2 LO-79-2: review provenance revisionとcurrent concurrency revisionの同一視

#### 識別

- 対象実装: 実装途中の`disposition_reviewed`と`release_resources`
- 直接の受益者: journal binding実装者（fieldと比較が少ない）
- コスト負担者: ledger進行後にcleanupするoperatorとshared resource利用者

#### 事実・推論・仮説

- [Evidence] reservation bindingはaccepted review revisionをimmutable provenanceとして保持する。
- [Evidence] 初期実装はrelease request baseをそのhistorical revisionと比較し、hostがcurrent case revisionをreplayしなかった。
- [Evidence] 修正後hostはjournal bindingからcase spaceを得てstoreをreplayし、request baseとcurrent revisionを完全一致させる（`src/bin/casegraphen-mcp-host.rs:359-389`）。allocatorはaccepted revisionをconcurrency tokenとして再比較しない。
- [Evidence] E2Eはreview reopen後、old accepted revisionを`stale_revision`で拒否しcurrent revisionでreleaseする（`tests/resource_host_e2e.rs:146-210`）。
- [Inference] provenanceとconcurrencyを同じrevision fieldで兼用すると、局所field数は減るがresource cleanupをledger lifecycleから切断する。
- [Hypothesis] long-running attempt中の多数revision更新でもcurrent replayとexplicit resubmitが許容運用負荷に収まる。

### 5.3 LO-79-3: reviewed/unreviewed mutationの同一public API所有

#### 局所的合理性と評価条件

- 局所目的: 同じatomic journal/replay/idempotency mechanicsをtest、pure library caller、operational hostで再利用する。
- 局所利益: wrapper型なしでfixtureが短く、resource protocol単体を永続評価できる。
- `B`: crate内部実装からpublic library/operational callerまで広げると、method名だけではauthority強度を保証できず優位性が反転する。
- `M`: API数の少なさから、誤用耐性とauthority vocabularyの正確さへ拡張。
- `N`: method renameだけでなくwrapper、tests、docsを同時変更。
- `T`: repository内callerだけから第三者Rust callerが増える期間へ拡張。

#### 四観測面と反実仮想

- 構造: 修正前は同じallocator valueからauthorityなし/ありのeventをpublicにappendでき、型がoperational guaranteeを表さなかった。修正後unreviewed methodsはprivateでwrapperだけが到達する。
- 実行: allocator mechanics testsは`UnreviewedResourceJournal`へ移り、host E2Eは`AtomicResourceAllocator::reserve_reviewed`だけを通る（`tests/resource_allocator.rs:1-70,276+`、`tests/resource_host_e2e.rs`）。
- 進化: 将来library callerが増えると「hostでは使わない」という規約だけではreview bypassの誤用レビューが反復する。
- 意味・組織: `AtomicResourceAllocator`はoperational authority、`UnreviewedResourceJournal`はmechanics evaluationという名前で責務を分ける。
- A: 同じpublic APIを維持しdocsで禁止する。誤用検出はreviewerへ外部化。
- B: methodを`reserve_unreviewed`へrenameするだけ。意図は明確になるがoperational valueから呼べる。
- C（採用）: operational allocatorのunreviewed mutationをprivate化し、明示wrapperへ隔離。追加wrapperとtest migrationが移行コスト。
- `Severity`: 7 / 15 (`E=2, A=1, F=2, K=1, T=1`)
- `Confidence`: C2（public API静的境界とtest caller migration）
- 判定: `externalization`、修正済み。

### 5.4 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| allocatorはcaller topologyを信頼 | arbitrary reviewedでないworkがshared resourceを占有 | operatorがclient入力を事前監査 | operator/other clients | reservationごと | Issue #79旧境界 |
| journalはresource recordだけ | runtime bundleがdeployment provenanceを証明不能 | callerがreservation/declarationを手動join | adapter/reviewer | resource-bearing runごと | 旧resource expectation contract |
| accepted revisionをrelease baseに流用 | ledger進行後のcleanup不能 | stale revision再送、手動journal介入 | operator/shared users | long-running attemptごと | 修正前review finding |
| superseding reservationは「reviewedならよい」 | unrelated deploymentがauthorityを置換可能 | operatorがbundle関係を目視 | operator/auditor | supersedeごと | 修正前branch、現`resource_allocator.rs:380-440` |

### 5.5 四観測面の証拠

#### 構造

- `ReviewedDeploymentAuthority`はprivate fieldsを持ちcanonical review+verified manifestからだけ生成され、allocatorはaudit用bindingへ投影する。
- authority constructorへ渡せるbundleもopaque `VerifiedDeploymentBundle`だけで、manifest/hashの形式一致をreview proofへ昇格できない。
- resource expectationは同じbindingを必須にし、hostはjournalのexact declaration/reservation/bindingと比較する（`src/bin/casegraphen-mcp-host.rs:272-306`）。
- releaseはbinding identityからcase spaceを得てcurrent storeをreplayする。supersedeは同じcase space、bundle、topology、policyを要求する（`src/resource_allocator.rs:380-440`）。

#### 実行

- reviewed reserve→runtime allocation→reconcile→`needs_review`のE2Eがあり、runtime outputは`accepted:false`を維持する。
- caller-supplied allocator snapshot、wrong claim/policy、stale compile、tampered artifact、stale releaseをfail closedで観測する。
- cross-deployment supersedeの専用実行反例は未追加であり、この枝のconfidenceは静的証拠に依存する。

#### 進化

- pure evaluatorはcaller snapshot問題を持ち、atomic allocator導入後もdeployment authority問題が残った。resource ruleを再実装せず、compiler proofをjournalへ投影することで境界を拡張した。
- binding追加はallocator event、runtime expectation、schema inventory、docs/Skillsの共変更を必要とし、authority語彙変更の増幅を可視化した。

#### 意味・組織

- bearer tokenはhost access、review proofはdeployment authority、allocator journalはresource authority、runtime reportはuntrusted observationである。修正後は同じ「許可」を一つのflagへ押し込まない。
- operatorの目視によるbundle/reservation対応付けをcanonical journal所有へ移した。

### 5.6 境界拡張と優位性反転

| 評価境界 | 旧案の利益 | 旧案のコスト | 代替案（review-bound journal）の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | caller topologyで直ちにgrant | authority不明 | proof引数と比較追加 | 型/branch増加 | 旧案 |
| モジュール | allocatorがledger非依存 | audit provenanceなし | opaque proofをplain bindingへ投影 | compiler型との結合 | 代替案僅差 |
| 機能 | resource conflictは防ぐ | unreviewed workもreserve可能 | review→reserve→reconcile join | bundle persistence必須 | 代替案 |
| システム | bearer authだけで簡単 | shared resource abuse/DoS | multi-client authorityをfail closed | store/artifact I/O | 代替案 |
| 運用・組織 | operator裁量が大きい | 手動照合・cleanup・監査 | journalがprovenance owner | refusal recovery手順 | 代替案 |
| ライフサイクル | eventが小さい | adapterごとjoin、stale cleanup | runtime family共通contract | journal growth/移行 | 代替案 |

- 反転する最小境界: allocator moduleからshared operational resource flowへ広げた時点。
- 反転する指標: grant atomicity単独からreview authority付きresource safety。
- 反転する時間軸: ledger revisionがreservation後に一度進み、cleanupが必要になった時点。

### 5.7 反実仮想

#### A. 現状維持（旧caller-topology予約）

- 定常コスト: operator/clientがreviewed deploymentとの対応を保証する。
- 将来コスト: runtime/tenantごとにadmission ruleが分岐する。
- リスク: authenticated arbitrary reservation、stale cleanup、runtime bundleの自己申告authority。

#### B. 最小限の局所改善

- 変更: reservation payloadへcaller supplied topology/review/bundle hashesを追加し、形式一致だけ検査。
- 利益: journal schemaだけでaudit情報を増やせる。
- 残る問題: callerが同じhashをコピーでき、review/store/artifact existenceを証明しない。
- 移行コスト: 小さいが名前だけ強いauthorityを作るため採用不能。

#### C. 境界をまたぐ構造変更（採用）

- 変更: reviewed compile bundle、verified artifact inventory、opaque authority、journal binding、runtime exact comparison、current revision release、same-deployment supersede。
- 成立条件: artifact rootとallocator journalがoperational hostのprivate durable stateであること。
- 定常利益: admission、runtime reconciliation、cleanupが同じauthority lineageを共有する。
- 新たなコスト: binding/schema bytes、store+bundle verification、journal replay、strict stale refusals。
- 移行の谷: historical unreviewed reservationsは新resource expectationで利用不能。experimental v0 fixture/consumerを同時更新する。
- ロールバック: operational hostをproposal/resource-disabledへ戻せる。既存journalはappend-onlyで保持し、authorityなしeventをaccepted runtime proofに使わない。

### 5.8 スコア

#### LO-79-1

- `E` 外部化コスト: 3
- `A` 変更増幅: 2
- `F` 境界障害: 2
- `K` KPI乖離: 2
- `T` 時間ロックイン: 2
- `Severity`: 11 / 15
- `Confidence`: C2

#### LO-79-2

- `E` 外部化コスト: 2
- `A` 変更増幅: 1
- `F` 境界障害: 2
- `K` KPI乖離: 2
- `T` 時間ロックイン: 1
- `Severity`: 8 / 15
- `Confidence`: C2

#### LO-79-3

- `E` 外部化コスト: 2
- `A` 変更増幅: 1
- `F` 境界障害: 2
- `K` KPI乖離: 1
- `T` 時間ロックイン: 1
- `Severity`: 7 / 15
- `Confidence`: C2

### 5.9 判定

- 分類: LO-79-1は`mixed`（authority costの`externalization`と運用security boundary）、LO-79-2/3は`externalization`。すべて修正済み。
- 判定理由: allocator内のatomic grantだけを最適化すると、deploymentの正当性とledger concurrencyがhost/operatorへ押し出され、shared operational flowで優位性が反転する。
- 反証となり得る情報: allocatorが単一trusted caller専用のlibraryで、resource admissionを上位ownerが必ず保証するなら旧`reserve`は`harmless-locality`である。operational hostは複数認証client向けsurfaceなので該当しない。
- 未検証事項: cross-deployment supersede E2E、multi-process contention、journal compaction、reservation TTL/quota、OS crash。
- 次に取得すべき証拠: independent clientsによるcontention/crash/release/supersede matrixとlong journal benchmark。

## 6. 横断的な補償構造

- 複数候補に共通する変換: review/compilerのopaque proofをjournal用plain audit bindingへ一度だけ投影する。
- 複数候補に共通する例外分岐: historical `Option<reviewed_deployment>`はdeserialize/replay互換のため残るが、operational resource bundleは`Some`を必須としfail closedする。
- 複数候補に共通する再試行・手動運用: stale revisionはcurrent再読取が必要。hostは自動`current`置換しない。
- 所有権・KPIに起因する再発構造: conflict-free grantだけをallocator KPIにすると、admission authorityとcleanup concurrencyが再び上位callerへ外部化される。

## 7. 候補ではなかったもの（false positives）

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| acceptance ledgerとallocator journalの二原本 | state重複 | ledgerはclaim/review、allocatorはactive resource stateを所有し、同じdecisionを二重実装しない。bindingはcontent join | bounded contexts / failure isolation |
| `UnreviewedResourceJournal`を残す | authorityなしreserveが依然存在 | 型名と別wrapperでmechanics-only trust boundaryを明示し、operational allocatorからunreviewed mutationへ到達不能 | intentional reusable lower layer |
| runtime expectationにもbindingを複製 | journalとの重複 | caller declarationをauthorityにせずjournal exact equalityを検査するtransport record | audit/reconciliation seam |
| full journal replay | O(n)で将来遅い | 現時点で実測反転がなく、append-only integrityを単純に保つ利益がある | `insufficient-evidence` time-delayed candidate |

## 8. 未検証事項

- cross-deployment / cross-case supersedeの実process negative fixture。
- 2 host processが同時reserve/releaseするcontentionとcrash-after-hard-link。
- 10万/100万event journal replayのp95/p99、startup/RSS。
- abandoned reservationのTTL、quota、operator recovery policy。
- artifact root/journal filesystem permissionsを破った攻撃者に対する保証（現在はprivate host stateが前提）。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | supersede authority matrix | unrelated bundle/case/attempt拒否 | 二つのreviewed deploymentを作るE2E negative test |
| 2 | multi-process crash/contention test | hard-link publicationとrelease atomicity | barrier付き子process、各publication点kill |
| 3 | long-journal benchmark | replay局所最適の反転点 | 1k/10k/100k/1m eventsでp50/p95/p99/RSS |
| 4 | retained remote resource pilot | runtime family間provenance保持 | issue-76 SQLite/async/process/file-dropを新bindingで再実行 |

## 10. 介入判断の前提

- 変更可能なチーム・サービス範囲: experimental compiler/host/allocator/runtime schemasとpilot adapters。
- 許容できる移行期間: v0期間中。historical journalは読み取るが新authorityには使用しない。
- 一時的に悪化してよい指標: reserve/release latency、event bytes、bundle verification I/O。
- 互換性・規制・SLO制約: append-only/hash-chain、create-new atomic publication、Rust 1.80、runtime outputはuntrusted。
- ロールバック要件: reviewed resource operationを停止可能であり、authorityなしeventをreviewedへ昇格しないこと。

## 11. 実装後の判定

反実仮想Cを採用した。operational reservationはopaque verified deployment bundleを必須とし、hostがstore/artifactからauthorityを再導出する。operational `AtomicResourceAllocator`のpublic mutationはreviewed pathだけで、authorityなしmechanicsは明示的な`UnreviewedResourceJournal`へ隔離した。allocator journalとruntime expectationが同じbindingを保持し、releaseはcurrent revision、supersedeはsame deployment authorityを検査する。調査した境界ではmaterialなauthority局所最適は残っていない。full replay性能は別の`insufficient-evidence`候補として維持する。
