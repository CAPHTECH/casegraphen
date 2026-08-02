# Issue #56 実装局所最適監査

## 1. エグゼクティブサマリー

- 調査範囲: canonical topology diff、audit/simulation evidence、redesign proposal、disposition history、accepted review binding
- 主要結論: auditからredesignを自動適用せず、exact diffとcontent-addressed unreviewed proposalをnormal reviewへ渡すpure moduleになった。監査でraw vector orderによるfalse diff/proposal identity drift、reviewer authorityの過小binding、hash rule重複を発見して修正した。
- 高確度候補: 修正済み3件。重大な未修正候補0件。
- 制約: production audit/review historyはなく、#55 deterministic simulation fixtureと静的証拠のみ。

## 2. システム成果とB/M/N/T

成果は `run trace → audit artifacts → exact redesign proposal → old/proposed simulation → normal review → accepted binding` を再構築可能にしつつ、accepted topology mutationを別の既存gateに残すことである。

| 変数 | 局所条件 | 拡張した条件 |
|---|---|---|
| `B` | node/edge comparison | topology version、audit/integration/expansion evidence、simulation、review ledger |
| `M` | diff生成量 | semantic stability、dedupe、authority、replay、情報損失 |
| `N` | pure diff module | #43 canonical hash、#45 lint、#47/#48/#54 IDs、#55 simulationを同時利用 |
| `T` | 一proposal | rejected/superseded/accepted history、次revision、反復redesign |

## 3. 証拠面

| 面 | 証拠 | 制約 |
|---|---|---|
| 構造 | `src/topology_redesign.rs`、strict schemas、中央hash/linter呼出し | 静的 |
| 実行 | 6 tests: canonical diff/order、evidence、authority、history、real simulation比較 | fixture |
| 進化 | append-only disposition hash chain | 実PR履歴なし |
| 意味・組織 | accepted bindingはreview ID/revision/authorityだけを記録しmutation APIなし | reviewer運用未観測 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | raw node/edge vectorをhashしてdiff | 実装が短い | canonical topology hashは同じでもfalse change | entity→version/replay | 11 | C2 | `mixed`、修正済み |
| 2 | evidence/set配列順をproposal IDへ含める | byte serializationが容易 | 同一redesignが複数IDになりreview/history分岐 | function→lifecycle | 10 | C2 | `time-delayed`、修正済み |
| 3 | accepted bindingのauthorityをnon-emptyだけで検査 | generic reviewer連携が容易 | proposalのrequired authorityを別authorityが満たしたように見える | API→governance | 12 | C2 | `externalization`、修正済み |

## 5. 詳細カードと優位性反転

### LO-56-1/2: 局所serializationとcanonical identity

- [Evidence] execution topology hashはnode/edge/policy配列とnested input/output/resource scopeをcanonicalizeする。
- [Evidence]初期diffはtyped entityをそのままJSON hashし、proposal materialもevidence/capability/uncertainty順を保持していた。
- [Inference] 一関数ではraw serializationが単純だが、version comparisonとreview dedupeへ境界を広げると優位性が反転する。
- [Fix] node input/output/resource claims/scopesとedge resource scopeをcanonicalizeし、proposalのset-like vectorsをsort/dedupeする。SHA-256は`native_hash::sha256_hex`だけを使う。

| 境界 | raw案の利益 | raw案のコスト | canonical案の利益 | canonical案のコスト | 優位 |
|---|---|---|---|---|---|
| 関数 | 最小実装 | なし | normalization追加 | code増 | raw |
| version | byte順でfalse change | review noise | semantic diff | order規則維持 | canonical |
| lifecycle | proposal ID分岐 | history/再review増幅 | stable dedupe | schema discipline | canonical |

### LO-56-3: authority binding

- [Evidence] proposalはauthority policy/capabilitiesを必須化するが、初期terminal dispositionは任意non-empty authorityを許した。
- [Fix] `AcceptedBinding.reviewer_authority_id`はproposalのauthority policy IDと一致し、normal review ID/revisionも必須。accepted binding後の再terminal dispositionは拒否する。
- 反転境界: generic log appendでは緩い案、governance/replayではexact binding案。

## 6. 反実仮想

- A: raw diff + auditから自動topology mutation。最短だがsemantic driftとacceptance bypassが大きい。
- B: canonical exact diff + unreviewed proposal + separate disposition binding（採用）。review工程とartifact管理は増えるがrollback可能で、既存gateを再利用する。
- C: redesign moduleがreviewとtopology store mutationまで所有。操作は一体化するがCaseGraphen decision ruleを複製し、移行/撤回が困難なため不採用。

スコアは LO-56-1 `E2/A3/F2/K2/T2=11 C2`、LO-56-2 `E2/A3/F1/K2/T2=10 C2`、LO-56-3 `E3/A2/F3/K3/T1=12 C2`。すべてBで修正・test済み。

## 7. 補償ハローと棄却候補

- exact Node/Edge/PolicyChange、uncertainty、information loss、simulation refsをproposal内に保持するため、reviewerが暗黙diffを再計算する補償を減らす。
- accepted bindingがmutationしないことは一工程増加に見えるが、acceptance-kernel境界として合理的であり局所最適候補から棄却した。
- old/proposed simulationは同じreportを流用せず、各topology hashへbindした#55 requestを二回実行する。

## 8. 未検証事項

| 優先 | 証拠 | 不確実性 | 方法 |
|---:|---|---|---|
| 1 | real #47 audit artifact→proposal trace | evidence IDの運用完全性 | E2E fixture/production sample |
| 2 | superseded chainを跨ぐ複数proposal | proposal間lineage | property test |
| 3 | simulation calibration drift | expected impactの予測精度 | planned-vs-actual audit |

介入を広げる場合も、accepted bindingからtopologyを直接変更せず、normal review revisionと既存mutation gateを要求する。simulation改善は推定でありaccepted factにしない。
