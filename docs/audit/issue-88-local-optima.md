# 実装局所最適監査レポート — Issue 88

## 1. エグゼクティブサマリー

- 調査範囲: content-addressed allocator checkpoint、suffix replay、archive compaction、MCP host保守経路、512-event pilot。
- 主要な結論: journal authorityを弱めずcheckpointを導入する局所目的は達成した。一方、(1) checkpointがevent index全体を複製すること、(2) request path上で独立検証とcompactionを同期実行すること、(3) journal locationをidentityへ永久結合することは、より長い運用境界で優位性が反転する候補である。
- 高確度候補数: 2件（LO-88-1、LO-88-2）、中確度1件（LO-88-3）。
- 証拠上の制約: 512-event debug pilotのみ実測済み。10k/100k、remote filesystem、長時間host、crash injectionは未実行。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- allocatorの排他・capacity・idempotency・reviewed authorityを保ったまま、長期journalの通常operation/restartをboundedにする。
- crash後も完全replayと監査回復を失わず、破損・別journal・別configurationをfail closedにする。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | allocator単体の正しさ | host request latency、backup/restore、運用者、release evidence |
| `M` 評価指標 | replay同値性・integrity refusal | p95 append、maintenance spike、checkpoint bytes、restore可用性 |
| `N` 変更可能範囲 | checkpoint/compaction module | host scheduling、archive format、migration protocol |
| `T` 時間軸 | 512 events・単一process | 10k/100k、複数年、host restart/relocation |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/resource_allocator.rs:769-1516`、schema、ADR 0026 | identity、checkpoint、archive、full/suffix replay | 静的読解 |
| 実行 | `docs/pilots/issue-88/resource-allocator-512.report.json` | 512 events、checkpoint、compaction、RSS | debug build、単一host |
| 進化 | Issue 88受入条件、旧issue-85 pilot | full replayだけの状態からの移行 | production履歴なし |
| 意味・組織 | `docs/guides/mcp-operational-host.md`、host CLI | operator policyとrequest lifecycle | 実SLO・担当分界未提示 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | LO-88-1 Full event index checkpoint | exact self-validation/idempotency | checkpoint growth、parse/hash cost | 10k+ lifecycle | 10 | C3 | 高確度候補 |
| 2 | LO-88-2 Synchronous host maintenance | simple atomic response semantics | interval requestのlatency spike/availability coupling | host feature | 9 | C3 | 高確度候補 |
| 3 | LO-88-3 Path-bound allocator identity | copied checkpoint substitution refusal | legitimate restore/relocation refusal | disaster recovery | 7 | C2 | 条件付き候補 |

## 5. 上位候補の詳細

### Candidate LO-88-1: Full event index checkpoint

#### 1. 識別

- 対象実装: `ResourceAllocatorCheckpointState.events_by_idempotency`と`validate_checkpoint_index`。
- 所有モジュール / サービス: `src/resource_allocator.rs`。
- 導入時期: Issue 88。
- 調査者: Codex implementation-local-optima audit。

#### 2. 事実・推論・仮説

- [Evidence] `src/resource_allocator.rs:1394` はcheckpoint内の全eventをsequence順に再hashする。
- [Evidence] 512 eventsでcheckpointは919,524 bytes、suffix replay 318 ms、compaction前full restartは102 ms（pilot lines 19, 23, 35）。
- [Inference] checkpointはprotocol decision replayを省くが、入力サイズをboundedにしない。
- [Hypothesis] 10k/100kではparse、clone、hash、RSSが支配し、通常operationの成長を十分抑えない。

#### 3. 局所的合理性

- 局所目的: checkpoint substitutionとderived-state不一致を単独artifact内で検出する。
- 局所指標: exact prefix/hash chain、idempotency replay、full replay equivalence。
- 直接の受益者: allocator integrity owner、監査者。
- 現在得られている利益: full grant/capacity validationなしでcheckpointを復元できる。
- 導入時の制約: authorityをjournalからcheckpointへ移してはならない。
- 現在も有効な制約: idempotency outcomeには過去eventの検索が必要。
- 失効した制約: なし。

#### 4. 評価条件

- `B`: checkpoint関数 → 長期host。
- `M`: tamper検出 → tamper検出＋bytes/latency/RSS。
- `N`: structだけ → indexed idempotency store/archive segmentも変更可。
- `T`: 512 → 100k events。

#### 5. 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| 全event複製 | checkpoint肥大化 | release-scale lane | release operator | 10k/100kごと | ADR 0026 promotion evidence |
| 起動時全index hash | suffix latency | retention interval調整 | host operator | 全request replay | 512 pilot |

#### 6. 四観測面の証拠

- 構造: derived stateとevent indexが同一checkpointに共存する。
- 実行: 512でsuffix 318 ms > pre-checkpoint full replay 102 ms。
- 進化: 旧full replay O(n)を解消する目的だがcheckpoint inputもO(n)。
- 意味・組織: promotion判定を10k/100k release laneへ外部化している。

#### 7. 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | 強い自己検査 | clone/hash | 同じ | 複雑 | 現在案 |
| モジュール | idempotency即時復元 | 大artifact | compact index | 追加schema | 同等 |
| 機能 | full replay保持 | request latency | Merkle/segment index | proof設計 | 未確定 |
| システム | 単純な一file | I/O/RSS | LSM型snapshot | components増 | 代替候補 |
| 運用・組織 | 説明容易 | scale lane負担 | bounded SLO | migration運用 | 反転候補 |
| ライフサイクル | 初期安全 | 線形成長 | versioned segmented checkpoint |移行谷 | 代替優位の可能性 |

- 反転する最小境界: 512を超える長期host（閾値は未測定）。
- 反転する指標: p95 operation/restart、checkpoint bytes、RSS。
- 反転する時間軸: 10k/100k evidence時。

#### 8. 反実仮想

##### A. 現状維持

- 定常コスト: O(events) checkpoint parse/hash。
- 将来コスト: checkpoint storageとmaintenance spike増大。
- リスク: promotion threshold未達。

##### B. 最小限の局所改善

- 変更: serialized event indexをcompact table化し、不要cloneを削減。
- 利益: bytes/CPU減。
- 残る問題: O(events)。
- 移行コスト: v0 schema破壊変更。

##### C. 境界をまたぐ構造変更

- 変更: immutable content-addressed journal segment＋Merkle root＋separate idempotency snapshot。
- 成立条件: segment verifierとfull replay equivalence。
- 定常利益: suffix costをsegment数/新規eventへ限定。
- 新たなコスト: index/segment recovery contract。
- 移行の谷: dual-readと旧checkpoint再生成。
- ロールバック: authoritative archived eventsからv0を再生成。

#### 9. スコア

- `E` 2、`A` 2、`F` 2、`K` 2、`T` 2、`Severity` 10/15、`Confidence` C3。

#### 10. 判定

- 分類: 実装局所最適候補。
- 判定理由: integrity局所目的は合理的だが、実測で性能目的との反転シグナルがある。
- 反証となり得る情報: release 10k/100kでSLO内かつcheckpoint growthが許容される。
- 未検証事項: release build、filesystem差、100k RSS。
- 次に取得すべき証拠: retained 10k/100k reports。

### Candidate LO-88-2: Synchronous host maintenance

#### 1–4. 識別・事実・合理性・評価条件

- 対象: `src/bin/casegraphen-mcp-host.rs:74-103,576,621`。
- [Evidence] interval到達request内でfull replay checkpoint、独立full verification、compactionを順次実行する。
- [Evidence] 512 pilotでは各処理333 ms、229 ms、449 msで合計約1.0秒。
- [Inference] exact operation responseへmaintenance結果を結合する実装は単純でfail closedだが、allocator decisionのavailabilityを保守I/Oへ結合する。
- 局所目的: background daemon/ambiguous completionを導入せず、保守完了をcallerへ明示する。
- `B`: request → host SLO、`M`: atomicity → p95/p99 availability、`N`: delegate → scheduler/lease、`T`: interval一回 → 継続運用。

#### 5–7. 補償ハロー・四面・反転

| 原因 | 影響 | 補償 | 負担者 | 証拠 |
|---|---|---|---|---|
| 同期full verification | interval request spike | interval調整/retry | client/operator | host implementation、pilot |

- 構造: decision pathとmaintenance pathが同一call stack。
- 実行: 512で約1秒のmaintenance work。
- 進化: durable hostは既にrequest idempotencyを持つため、maintenance jobの独立化余地がある。
- 意味・組織: clientはresource decisionと保守失敗を同じrefusalとして受ける。
- 最小反転境界: operational hostのrequest SLO。

#### 8. 反実仮想

- A 現状維持: 単純・安全。interval p99 spikeとdisk incident couplingを受容。
- B 局所改善: maintenance budget/timeoutを設定。中断点が増える。
- C 構造変更: committed allocator event後にcontent-addressed maintenance intentをenqueueし、single-owner workerが実行。成立条件はcrash-resume/idempotency。移行の谷はdual mode、rollbackは同期modeへの設定切替。

#### 9–10. スコアと判定

- `E` 2、`A` 2、`F` 2、`K` 2、`T` 1、`Severity` 9/15、`Confidence` C3。
- 分類: 高確度候補。反証: 実SLOが1秒超を許容しmaintenance failureをallocator failureとして扱う契約が承認済みであること。

### Candidate LO-88-3: Path-bound allocator identity

#### 1–10. 要約カード

- [Evidence] `src/resource_allocator.rs:1504-1516` はcanonical directory path hashをidentityへ結合する。
- 局所合理性: journal/checkpoint directory copyによるcross-journal authority reuseを確実に拒否する。
- 補償ハロー: backup restore、volume mount変更、disaster recoveryはすべて拒否され、将来の明示migration toolが必要。
- 四面: 構造上pathがauthority、実行testはcross-directory copy refusal、進化上migration未実装、組織上restore operatorへ負担。
- 反転: 単一hostでは現在案優位、DR lifecycleではportable instance-id＋signed migration recordが優位になり得る。
- A: 現状維持。B: configured stable journal IDを追加。C: source/destinationを結ぶreviewed migration recordとone-time rebind。移行の谷は旧identityと新identityのdual verification、rollbackは元pathを保持。
- `E` 2、`A` 1、`F` 2、`K` 0、`T` 2、`Severity` 7/15、`Confidence` C2。
- 判定: 条件付き候補。DR要件が「同一path復元のみ」なら反転しない。

## 6. 横断的な補償構造

- 共通する変換: full replayがcheckpoint、compaction、recoveryの最終oracleになる。
- 共通する例外分岐: integrity mismatchは一律fail closed。
- 共通する再試行・手動運用: scale pilot、interval設定、将来migrationがoperatorへ残る。
- 所有権・KPIに起因する再発構造: security/integrity KPIがrequest latencyとDR portabilityより優先される。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| archive eventを削除しない | compaction後もstorage増 | full replay authority保持が明示要件 | destructive retentionは別reviewが必要 |
| pending fileを無視 | partial bytesを見ない | create-new publication前はauthorityがない | crash safety境界として正しい |
| normal replayとfull replayを分離 | normal pathがarchive bytesを毎回再hashしない | checkpoint stateだけを使用し、full auditは明示APIで全bytes検証 | latencyと監査を区別した意図的境界 |
| opaque verified checkpoint proof | public callerが作れない | compaction authorityの必要条件 | caller-constructible proofを防ぐ |

## 8. 未検証事項

- 10k/100k release benchmarkとpromotion threshold。
- process killを各fsync/hard-link/remove境界へ注入した実crash test。
- NFS/network volumeのhard-link、directory fsync semantics。
- host concurrency下のmaintenance p99。
- reviewed reservation authorityを含むhost E2Eでのcheckpoint前後同値性。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | 10k/100k retained report | LO-88-1反転点 | release pilot専用host |
| 2 | interval前後host latency histogram | LO-88-2外部化コスト | independent MCP client load |
| 3 | kill-point matrix | crash safety | subprocess fault injection |
| 4 | restore drill | LO-88-3の実害 | copied volume＋reviewed migration design |

## 10. 介入判断の前提

- 変更可能な範囲: experimental v0 schema、allocator、host lifecycle。
- 許容できる移行期間: archived journalを保持したdual-read期間。
- 一時的に悪化してよい指標: release pilot時間。authority semanticsは悪化不可。
- 互換性・SLO制約: full replay、idempotency、reviewed authority、fail-closedを維持。
- ロールバック要件: archive bytesから旧checkpointを再生成できること。
