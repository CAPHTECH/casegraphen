# 実装局所最適監査レポート — Issue 88

## 1. エグゼクティブサマリー

- 対象: allocator checkpoint、suffix replay、compaction、hot replay cache、writer lock、MCP host、512/10k/100k release-scale evidence。
- 局所的合理性: append-only journalを毎操作full replayする旧実装は最も単純で強いintegrity oracleだった。checkpointも全履歴indexを保持することで、idempotencyとprefix検証を一artifactで復元できた。
- 結論: 毎操作full replay、全履歴clone、proposal件数ベースの測定という既知の局所最適は解消した。clean exact commit `9b23383463cb1f1fafb666e7fb87a596b3e090e2`から生成した10k/100k public-API laneはいずれも設定threshold内で、両laneとも10,000 shared-read all-activeとmixed churnを通過した。
- 残る候補: checkpoint全event indexの線形成長、同期maintenance、path-bound identity。100k実測はpromotion budget内だが、約3.75GB RSS、約181.8MB checkpoint、約41.9秒compactionという外部化コストも定量化した。production request SLO、さらに長いjournal、DR境界では優位性が反転し得る。
- Promotion authority: retained 10k/100k reportsはclean exact-revisionのrelease-candidate evidenceだが、attestationはなく、両方とも明示的に`promotion_authority: false`である。

## 2. システム成果と評価条件

### 最終成果

allocatorの排他、capacity、idempotency、reviewed deployment authorityを変えず、長期journalの定常appendとrestartをboundedにし、crash・tamper時にfull audit recoveryを維持する。

### 評価条件

| 変数 | 旧い局所条件 | 拡張した条件 |
|---|---|---|
| `B` 評価境界 | 一allocator関数 | operational host、別process writer、operator、release evidence、DR |
| `M` 評価指標 | replay同値性 | authority同値性、append p95、restart、RSS、checkpoint bytes、busy上限 |
| `N` 変更可能範囲 | replay関数 | cache/index、operation response、lock contract、host lifecycle、evidence harness |
| `T` 時間軸 | 512 events | 10k/100k、process crash/restart、長期archive、relocation |

## 3. 証拠

| 観測面 | 証拠 | 観測 | 制約 |
|---|---|---|---|
| 構造 | `src/resource_allocator.rs` | durable event後だけcache更新、identity/head/next-sequence/maintenance inventoryでinvalidate、full auditを別経路に保持 | middle-eventをhot pathで毎回rehashしない |
| 実行 | `tests/resource_allocator.rs` | real subprocess lock timeout/crash/contention、hint failure後のcommit、identity reuse refusal、hot tamper/restart refusal | local filesystem |
| 実行 | `resource-allocator-512.report.json` | immediate release、128 all-active、128 mixed churn、checkpoint/full equivalence | bounded lane |
| 実行 | `resource-allocator-10000.report.json` | append 101.5s、restart 401ms、all-active 10,000件100.1s、mixed 1,024 pair 20.6s、RSS約453MB、equivalence=true | clean commit、unattested、promotion authorityなし |
| 実行 | `resource-allocator-100000.report.json` | append 1,040.2s、restart 4.9s、all-active 10,000件100.9s、mixed 4,096 pair 82.6s、RSS約3.75GB、equivalence=true | clean commit、unattested、promotion authorityなし |
| 進化 | Issue #88 acceptance criteria、ADR 0026 | full replay per appendからcheckpoint＋hot cacheへ移行 | production historyなし |
| 意味・組織 | MCP host guide、pilot README | private service identity、non-authoritative hint、operator review、nonpromotion evidenceを明示 | production SLO未承認 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | LO-88-1 Full event index checkpoint | exact prefix/idempotency復元 | bytes・parse・hash・RSSの線形成長 | 100k超 / 長期fleet | 9/15 | C3 | 高確度候補 |
| 2 | LO-88-2 Synchronous maintenance | 単純な完了意味、曖昧なbackground effectなし | interval request latencyとavailabilityの結合 | host p99 SLO | 8/15 | C3 | 高確度候補 |
| 3 | LO-88-3 Path-bound identity | copied journal substitution拒否 | legitimate restore/relocation refusal | DR lifecycle | 7/15 | C2 | 条件付き候補 |
| 4 | LO-88-4 Hot-cache filesystem trust boundary | O(1)定常decision | out-of-band middle-byte tamperはrestartでのみ検出 | shared writable volume | 6/15 | C3 | 制約付き非候補 |

## 5. 詳細カード

### LO-88-1: Full event index checkpoint

#### 事実・推論・仮説

- [Evidence] checkpointは`events_by_idempotency`を含み、独立verificationはjournal prefixを完全replayする。
- [Evidence] 10k checkpointは約18.1MB、create 587ms、verify 504ms、compaction 3.8s、RSS約453MB。
- [Evidence] 100k checkpointは約181.8MB、create 6.5s、verify 5.6s、compaction 41.9s、RSS約3.75GB。全項目は明示budget内だった。
- [Inference] 定常appendはboundedになったが、checkpoint sizeとmaintenanceはeventsに比例する。
- [Inference] 10kから100kへの増加はcheckpoint bytes、maintenance、RSSの線形成長を実測で確認したが、100k promotion budgetは超えていない。
- [Hypothesis] 100k超またはより厳しいproduction SLOではbytes/RSSまたはcompaction時間がfleet SLOを支配し得る。

#### 局所的合理性

目的は、journal authorityをcheckpointへ移さず、restart時にidempotency replayとactive stateを復元することだった。全event indexはschema追加を最小化し、full replayとのexact comparisonを容易にする。導入時も現在も、過去idempotency keyのcollision判定は必要である。

#### 補償ハロー

| 局所判断 | 境界外影響 | 補償 | 負担者 | 規模 |
|---|---|---|---|---|
| 全event index | checkpoint/RSS線形成長 | 10k/100k evidence lane | release operator | releaseごと |
| full independent verify | maintenance spike | interval/retention調整 | host operator/client | checkpoint intervalごと |

#### 境界拡張

| 境界 | 現在案 | 代替案 | 優位性 |
|---|---|---|---|
| 関数 | exact self-validation | compact indexは複雑 | 現在案 |
| モジュール | restartが単純 | segmented index | 現在案 |
| host | maintenance spike | background intent/worker | 未確定 |
| fleet lifecycle | artifact線形成長 | immutable segment＋Merkle/index snapshot | 反転候補 |

100k retained evidenceでは現在案の優位性は反転しなかった。次の反転候補境界は100k超のjournalまたはhost p99 SLOであり、指標はcheckpoint bytes、RSS、verify/compaction latencyである。

#### 反実仮想

- A 現状維持: authority説明とrollbackは最も簡単。線形成長を受容する。
- B 局所改善: compact idempotency table、clone削減、checkpoint compression。O(n)は残る。
- C 構造変更: immutable journal segments＋Merkle root＋separate idempotency snapshot。suffixは新規segmentだけ検証する。dual-read migrationとsegment recovery contractが必要。archive bytesから旧v0を再生成できることをrollback条件とする。

#### スコアと判定

`E=2 A=2 F=2 K=1 T=2`、Severity 9/15、Confidence C3。100kで線形成長は確認されたがbudget内だったため、直ちに構造変更すべきという強い主張は反証された。production SLOと100k超の証拠が次の判定条件である。

### LO-88-2: Synchronous host maintenance

#### 事実と合理性

- [Evidence] interval到達request内でcheckpoint、independent verify、compactionを順次実行する。
- [Evidence] 10kでは合計約4.9s、100kでは合計約53.9sのcheckpoint create・independent verify・compaction work。
- [Inference] decision commitとmaintenance failureを同じrequest lifecycleへ結合する。

background daemonを導入せず、callerへmaintenance完了を正確に返す点は局所的に合理的である。一方、allocator decisionのavailabilityを保守I/Oへ結合する。

#### 補償ハロー・反転

| 原因 | 影響 | 補償 | 負担者 |
|---|---|---|---|
| 同期full verification | interval request spike | retry、interval調整 | client/operator |

最小反転境界はoperational hostのp99 SLO。Aは同期維持、Bはmaintenance budget/typed deferred result、Cはdurable content-addressed maintenance intentをsingle-owner workerが実行する構造。Cはcrash-resume/idempotency proofが必要。

`E=2 A=2 F=2 K=1 T=1`、Severity 8/15、Confidence C3。

### LO-88-3: Path-bound allocator identity

- [Evidence] canonical journal pathをallocator identityへ結合し、別directoryへのcopyを拒否する。
- 局所利益: cross-journal checkpoint substitutionを明確に閉じる。
- 外部化: volume mount変更、restore、DRで明示migrationが必要。
- A: 現状維持。B: configured stable instance ID。C: source/destinationを結ぶreviewed one-time migration record。
- 反転境界: DR drill。元pathとarchiveを保持することがrollback条件。
- `E=2 A=1 F=2 K=0 T=2`、Severity 7/15、Confidence C2。

### LO-88-4: Hot-cache filesystem trust boundary

- [Evidence] hot cacheはidentity、head、next sequence、checkpoint/compaction inventory変化でinvalidateする。
- [Evidence] older eventのunsupported in-place tamper後、hot stateは既導出decisionを維持し、fresh/full replayは拒否する。
- 局所利益: 毎operation historical bytes rehashという旧O(n)を復活させない。
- 境界: private service identity/canonical writersだけがjournalへ書く。
- 判定: 現契約内では局所最適候補ではない。shared writable journalを許す要求が生じた場合のみ、external integrity monitor、signed filesystem events、またはMerkle proofへ再設計する。
- `E=1 A=0 F=1 K=2 T=2`、Severity 6/15、Confidence C3。

## 6. 解消した局所最適

| 旧判断 | 局所利益 | 反転 | 修正 |
|---|---|---|---|
| 毎append full replay | 最も単純なauthority check | 512件で累積36s | validated long-lived cache＋checkpoint suffix replay |
| appendごと全history clone | responseが完全snapshot | all-activeでO(n²) | countだけのbounded operation snapshot、full snapshotは明示API |
| unbounded advisory lock | single writerが単純 | crashed/stalled writerでavailability消失 | 500ms typed `WriterBusy`＋process crash test |
| hint failureをappend error化 | hint同期を強制 | durable event後のfalse failure/retry | committed success＋`head_hint_healthy:false` |
| proposal数中心のscale証拠 | harnessが短い | active-set clone/scanを見逃す | all-active＋mixed churn public-API lanes |

## 7. 横断的な補償構造

- authority oracleはfull replayに集約され、checkpoint/cacheはacceleratorに限定される。
- performance compensationはrelease-scale laneとhost retention設定へ現れる。
- security ownerのfail-closed KPIが、latency ownerとDR operatorへmaintenance/migration負担を外部化する構造がある。
- operation responseをboundedにしたため、complete active inventoryが必要なcallerは明示snapshot/pagination設計を選ぶ必要がある。

## 8. 候補ではなかったもの

| 対象 | シグナル | 棄却理由 |
|---|---|---|
| archive bytesを削除しない | storage増 | full audit authority保持という明示要件。destructive retentionは別review |
| pending fileを無視 | partial dataを見ない | create-new publication前はauthorityではない |
| head hintをauthorityにしない | replay増 | hint failure後もdurable eventを失わないため正しい |
| historical reservation identity再利用拒否 | ID枯渇 | retry/alias attackを防ぐcanonical identity invariant |
| release-candidate reportをpromotion不可にする | releaseを遅らせる | clean sourceでもunattested hostからauthorityを作らない正しい意味境界 |

## 9. 未検証事項

- provider/host attestationを伴うpromotion lifecycle。
- 100k超または長期反復時のcheckpoint/RSS成長。
- remote/network filesystemのhard-link、directory fsync、advisory lock semantics。
- host concurrency下でのmaintenance p95/p99。
- migration/restore drill。
- checkpoint format/compiler version migration。

## 10. 次に取得すべき証拠

| 優先 | 証拠 | 解消する不確実性 |
|---:|---|---|
| 1 | host interval前後latency histogram | LO-88-2の約53.9秒maintenanceがrequest p99へ与える影響 |
| 2 | attested retained evidence lifecycle | release-candidateからpromotion authorityへの境界 |
| 3 | 100k超または反復long-journal report | LO-88-1の次の反転点 |
| 4 | remote filesystem fault matrix | lock/fsync/hard-link portability |
| 5 | restore/migration drill | LO-88-3の実害と移行の谷 |
| 6 | checkpoint version migration fixture | lifecycle compatibility |

## 11. 介入判断の前提

- experimental v0のRust/MCP responseは明示migration可能。
- authority semantics、full replay、idempotency、review bindingは悪化不可。
- performance改善のため一時的にrelease evidence時間は増加してよい。
- rollbackはauthoritative archiveから旧checkpoint/stateを再生成できること。
- 今後100k超またはproduction p99がthresholdを超えた場合、threshold緩和ではなくLO-88-1/2の構造代替を先に評価する。
