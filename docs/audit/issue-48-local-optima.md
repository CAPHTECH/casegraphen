# Issue #48 実装局所最適監査

## 1. エグゼクティブサマリー

- 調査範囲: generic JSONL ingest、runtime protocolとのjoin、artifact inventory、proposal/review seam、consumer Skill
- 主要な結論: runtime reportをaccepted truthへ昇格せず、canonical completenessを`runtime_protocol`へ委譲している。監査で「report joinがcompleteなら、宣言されたoutput bytesが未観測でもproposal可能」という境界反転を発見し、integration固有のinventory検査でhaltするよう修正した。
- 高確度候補数: 修正済み1件。残る弱い候補2件。
- 証拠上の制約: 新規in-memory adapterのfixture実行のみ。長時間fleet、永続化、binary artifact、運用復旧は未計測。

## 2. システム成果と評価条件

成果は、外部runtimeのJSONLを厳密・冪等に取り込み、欠落を完成扱いせず、content-addressed artifactからunreviewed proposalだけを生成してreview seamで停止することである。

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` | report reconciliation | artifact bytes、ledger review seam、consumer Skill |
| `M` | protocol completeness、単純なingest | evidence provenance、復旧性、長期memory、operator負荷 |
| `N` | adapter module | runtime protocolは変更せずadapter/schema/Skill/testを変更可能 |
| `T` | 一回のJSONL batch | incremental replay、長時間run、process restart |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/runtime_integration.rs`, `src/runtime_protocol.rs`, JSONL schema | validation、dedupe、join、proposal | 静的 |
| 実行 | `tests/runtime_integration.rs`, `tests/casegraphen_integrate.rs`, install smoke | omit-one、追加、retry、hash、missing bytes、Skill | fixture |
| 進化 | shared diff | 今回のみ | historyなし |
| 意味・組織 | Skill境界とtyped halt | runtime/integrator/reviewerの責務 | operator調査なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | protocol completenessをartifact presenceと同一視 | 既存canonical ruleだけで完結 | reviewerへ実体のないevidence proposal | report graph→evidence ingest | 11 | C2 | `externalization`、修正済み |
| 2 | stateをmemoryへ保持しreconcile時clone | 最小adapterが単純 | long run/restartでmemory・再投入 | fixture→fleet lifecycle | 7 | C1 | `time-delayed`候補 |
| 3 | invalid line findingをinstance lifetimeで保持 | fail-closedで欠落を隠さない | 修正後の継続には明示的再構築が必要 | batch→operator recovery | 6 | C1 | `externalization`候補 |

## 5. 上位候補カード: LO-48-1

### 事実・推論・仮説

- [Evidence] canonical `RuntimeCompleteness`はreport join、schema、retry、missing report、observed-but-unaccounted artifactを評価するが、reportが宣言したoutput IDのbytesがinventoryにあることまでは表さない。
- [Evidence] regression testは全node reportが揃いprotocol completenessがtrueでも、output bytes未ingestなら`missing_declared_artifact`、proposalなし、`incomplete_runtime_reports` haltを確認する。
- [Inference] protocol boundaryでは合理的だが、evidence proposal境界では優位性が反転する。
- [Hypothesis] 外部runtimeがbytes upload前にreportを送る頻度は未計測。

### 局所的合理性

- 局所目的: completeness ruleを一箇所に保ち、adapterがretry/schema/joinを再実装しない。
- 直接の受益者: protocol保守者とruntime adapter実装者。
- 現在も有効な利益: completeness本体は依然`reconcile_runtime_reports`だけが所有する。
- 失効した制約: なし。修正は別の判断である「提案可能なbytes inventory」に限定した。

### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 規模 | 証拠 |
|---|---|---|---|---|---|
| report joinだけでproposal可能 | artifact実体なし | reviewerが手動確認/再取得 | reviewer/operator | output数比例 | regression fixture |

### 優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| protocol | 一つのcanonical completeness | bytes transportを知らない | protocolへbytes rule追加 | 責務混入 | 現在案 |
| integration | rule reuse | 実体なしproposal | adapter inventory gate | 追加finding | 代替案 |
| ledger/review | proposal生成が容易 | review不能 | bytes存在を保証 | upload待ち | 代替案 |

### 反実仮想と修正

- A 現状維持: protocol completeをそのままproposal gateにする。
- B 最小改善（採用）: completenessは変更せず、adapterがcontent-addressed output bytesの存在をproposal前提として検査する。
- C 構造変更: runtime protocolへartifact manifest/transportを統合する。責務が広がり、他adapterを拘束するため不採用。

`E=3, A=1, F=3, K=3, T=1`, Severity `11`, Confidence `C2`, 判定 `externalization`。Bを適用しtestで再検証した。

## 6. 残る候補

### LO-48-2: in-memory state

`BTreeMap`によるexact replay dedupeは小規模fixtureで監査しやすい。一方、report/artifact bytesをrun全体で保持し、reconcileでreportをcloneするため、長時間fleetではmemoryとrestart再投入が負担になり得る。A現状、B append-only local store、C external object store + durable ingest ledgerを比較すべきだが、実測・P1 resource/control-plane contractなしに永続化しない。Severity `7`, Confidence `C1`, `time-delayed`候補。

### LO-48-3: sticky invalid findings

invalid lineをinstance lifetimeで保持するため、不正入力を後から黙って消せない局所的安全性がある。反面、operatorは新instanceへvalid streamをreplayして回復する必要がある。A sticky、B explicit reset with audit record、C durable disposition ledgerが候補。現在はSkillへfail-closedを明記し、reset APIを安易に追加しない。Severity `6`, Confidence `C1`, `externalization`候補。

## 7. 候補ではなかったもの

| 対象 | シグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| nested node report schemaをJSONL schemaへ複写しない | schemaが緩く見える | Rustは唯一の`parse_runtime_node_report`で厳密検証 | rule duplication防止 |
| proposalが常にunreviewed/accepted false | extra review負荷 | 製品のacceptance kernel境界 | 非交渉trust seam |
| UTF-8 content限定 | binary非対応 | v0が暗黙base64を避け、参照文書で制約明示 | bounded experimental contract |

## 8. 未検証事項と次の証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | report/artifact 200/1000/10000件のmemory/latency | LO-48-2 | benchmark |
| 2 | process crash/resume fixture | durable ingestの必要性 | P1 control-plane spike |
| 3 | invalid batchからのoperator recovery時間 | LO-48-3 | Skill behavior eval |

## 9. 介入判断の前提

- retry/schema/join/completenessは`runtime_protocol`以外へ複製しない。
- durable化してもcallerの`base_revision_id`をcurrentへ置換しない。
- artifact bytesを保存先へ移す場合もcontent hashを再検証し、proposalはreviewまでunreviewedを維持する。
- adapterはscheduler、retry engine、model callerを所有しない。

## 10. Cross-issue resource integration correction

#44のcompactなallocation summaryだけを保持する設計は、node report単体では
runtime metadataを小さく保つ局所的利益があった。しかし`B`を#50の
declaration/reservation/actual reconciliationまで、`M`をreport completeness
からresource safetyまで、`N`をruntime adapterとresource protocolへ、`T`を
review proposal生成まで広げると、allocation mismatchが検査されずreviewerへ
外部化される優位性反転が確認できた。

構造証拠はcompact summaryと完全な`RuntimeResourceAllocation`の型差、実行証拠
はsubstituted resourceを含む完全report集合がproposalを生成しない統合テスト
である。A: summaryを信頼、B: adapter内にresource比較を再実装、C: 別のtyped
JSONL allocation recordを取り込み、#50の`reconcile_resource_allocations`へ委譲
する案を比較しCを採用した。Cはcallerにtopology-bound declaration/reservation
の提示を要求するが、missing/orphan/mismatchをすべてfail closedにし、decision
ruleを複製しない。判定`externalization`（修正済み）、`E=3,A=2,F=3,K=3,T=1`、
severity `12/15`、confidence `C2`。永続ingestと複数process atomicityは依然
#52 adapterの未検証責務である。
