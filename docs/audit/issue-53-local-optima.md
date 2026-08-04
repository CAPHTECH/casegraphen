# Issue #53 実装局所最適監査

## 結論

streaming reconciliationをB/M/N/Tで監査し、arrival orderの局所的な低遅延をaccepted logへ持ち込む優位性反転を除去した。重大な未修正候補はない。

## B/M/N/Tと証拠

| 変数 | 局所 | 拡張後 |
|---|---|---|
| B | event handler | topology/resource/acceptance/replay/final completeness |
| M | 最初のchunkからの遅延 | 決定性、欠落可視性、trust、resource safety |
| N | 配列順処理 | external runtime protocolとreconciler。core log ruleは不変 |
| T | 一配送 | duplicate/delayed replay、partial run、後日のaudit |

構造証拠はADR 0024と、terminal判定を唯一の`reconcile_runtime_reports`へ委譲するmodule境界。実行証拠はout-of-order/duplicate、slow sibling、gate/resource block、completeの4反例。進化証拠はexperimental schema、意味/組織証拠は全releaseが`accepted=false`でruntime scheduler責任を保持する点である。

## 候補と反実仮想

| Candidate | 局所利益 | 外部化 | Severity | Confidence | 判定 |
|---|---|---|---:|---|---|
| arrival/completion orderをappend順にする | 最小latency/実装量 | replay差、readiness差、監査不能 | 13 (E3/A3/F3/K3/T1) | C2 | inversion、除去済み |
| chunkだけでdownstreamを解放 | pipeline幅 | resource衝突とreview bypass | 13 (E3/A3/F3/K3/T1) | C2 | externalization、除去済み |
| partial progressをcomplete扱い | UIが単純 | slow/missing siblingを隠す | 11 (E3/A2/F3/K2/T1) | C2 | externalization、除去済み |

Aはcompletion order適用、Bはfrontier barrier維持、Cはruntimeだけpipelineしlogical orderでreconcileする案。Cは局所状態を増やすが、run/replay/acceptance境界で優位。release proposalはtyped data edge、streaming、reservation、acceptance gateの積集合に限定した。

## 残存不確実性

logical order allocatorのmulti-runtime運用、chunk byte ingestとのatomicity、大規模event集合のmemoryは未実測（C1）。次の証拠は#52 transport replayと10万event benchmark。最適化はstable orderingとterminal completeness delegationを変更しないことを条件とする。

## 独立post-audit correction

### 調査条件と証拠

独立レビューでは `B` をevent sortingからtopology deployment/resource
reservation/downstream releaseまで、`M`を再現性からresource safetyと
equivocation拒否まで、`N`をreconciler単体から#43 topology/#50 resource
protocolとの同時変更まで、`T`を一配送からgap解消・retry・replayまで
広げた。構造証拠はtyped inputとjoin条件、実行証拠は追加した
topology-hash mismatchおよびsequence/chunk collision反例、意味証拠は
early releaseが未受理でも外部runtimeのdownstream実行を誘発し得る点で
ある。production transportとresource operatorの進化/運用証拠はない。

| Candidate | 局所的合理性 | 境界外コスト | 反転境界 | Severity | Confidence | 判定 |
|---|---|---|---|---:|---|---|
| caller booleanでresource許可 | APIが小さく高速 | canonical reservation/allocation不一致をconsumerへ移す | handler→resource system | 12 (E3/A2/F3/K3/T1) | C2 | externalization、修正済み |
| eventはexpectationへjoinするがtopology自体はjoinしない | event検査だけで完結 | stale/different topology edgeでrelease可能 | event→deployment | 12 (E3/A2/F3/K3/T1) | C2 | externalization、修正済み |
| sequenceをdedupeしてstable sort | replay outputが決定的 | 同一sequenceのequivocationを正常化しrelease | sort→runtime side effect | 13 (E3/A3/F3/K3/T1) | C2 | mixed、修正済み |

修正後はdownstream nodeごとのcanonical `ResourceReconciliation`が
`complete`かつfindingなしであること、topology id/content hashがexpectation
へjoinすることを要求する。同一attempt sequenceやchunk indexの衝突、
chunk gap/final位置不整合が一つでもあればcanonical prefix未確立として
全early releaseを止める。terminal completenessのownerは変更していない。

反実仮想は、A: 既存のboolean/stable-sort維持、B: canonical joinと
fail-closed prefix gate（採用）、C: streaming transportをcore schedulerへ
統合、の三案を比較した。Aは局所実装量のみ優位だがruntime side effect
境界で反転する。Cはmessage bus/retry/schedulerをcoreへ持ち込む移行の谷と
新しいdecision-rule重複が大きい。Bはresource reconciliation生成とgap待ち
をcallerへ要求するが、既存protocolを再利用しrollback可能である。

残存リスクは、node→resource reconciliationの対応付けがhost integration
入力であること、active/superseded producer attemptのcanonical permitが
stream contractにまだないこと、chunk bytes/hashの一致が#48 integrator
まで観測不能なことである。後二者はC1、推定Severity 7であり、#52の
stateful control plane traceを次の証拠とする。

実装後のschema round-tripで、flattenしたtagged payloadと外側の`deny_unknown_fields`が衝突し、正しい`kind`まで拒否する局所最適を検出した（E2/A1/F2/K2/T1=8、C3）。A: strictnessを外して全fieldを許す、B: nested payloadへ破壊変更、C: tagged payload側でunknown fieldを拒否し外側はflattenへ委譲、を比較してCを採用した。canonical exampleの往復と権威を装う未知fieldの拒否を同じテストで固定した。

横断監査で、`acceptance_satisfied_edge_ids`がcaller自己申告でreview/evidence gateを解除できる欠陥を検出した（E3/A3/F3/K3/T1=13、C3）。A: ID setを信頼、B: adapterごとに検査、C: `evaluate_native_case`だけが構築できるopaque readiness projectionを同一topology hashへjoin、を比較してCを採用した。これによりstream reconcilerはacceptance ruleを再実装せず、gated targetのwork cellがcanonical evaluatorでreadyな場合だけ解放する。

さらにresource側も、canonical reconciliationをcallerが任意のnode keyへ格納する
Mapでは別graph/nodeのcomplete recordを流用できる局所最適を検出した。局所的には
Map lookupが最小APIだが、`B`を#48 integration/#50 topology-bound reservation/
downstream dispatchへ、`M`をcomplete flagからidentity-safe allocationへ、`N`を
streamingとruntime integrationの同時変更へ、`T`をearly dispatch attemptまで広げる
と優位性が反転する。構造証拠はreconciliation recordにnode/topology associationが
なくcaller keyだけで補っていた点、実行証拠はcross-graphとcross-node substitution
反例である。

A: caller Mapを維持、B: stream reconcilerでresource ruleを再実装、C: exact topology
id/hash/node/claim expectationと#48が出力したcanonical reconciliationの
declaration/reservation/attempt joinからのみopaque `StreamingResourcePermits`を構築、
を比較してCを採用した。early release proposalはpermitが保持するtarget attempt IDも
明示し、別attempt dispatchを防ぐ。`E=3,A=3,F=3,K=3,T=1`、severity `13/15`、
confidence `C2`、判定`mixed`（externalization + time-delayed、修正済み）。残る
運用不確実性はintegration report自体のdurable provenanceであり、#52外部adapterの
transaction boundaryで検証する。
