# Issue #51 実装局所最適監査

## 1. 結論

VerificationPolicy、quorum、anchor照合を実装した。B/M/N/Tを拡張した結果、runtime metadataを独立性の証明として扱う局所最適と、policy照合をcore review ruleへ混入する局所最適を設計段階で除去した。未修正の重大候補はない。

## 2. B/M/N/T

| 変数 | 局所条件 | 拡張条件 |
|---|---|---|
| B | policy objectのvalidation | producer→verifier→world anchor、既存acceptance seam |
| M | actor差・quorum成立 | 観測可能性の誠実さ、decision rule非重複、将来adapterの補償量 |
| N | 新規experimental module/schema | core review/evidence ruleは変更しない |
| T | v0 fixture | 複数runtime/anchorで語彙を検証してstable化するまで |

## 3. 証拠面

- 構造: `verification_policy.rs`は既存review/evidence mutationを呼ばずpolicy resultだけを返す。
- 実行: same actor、3者quorum、failed anchor、runtime attestationのunit反例。
- 進化: experimental schemaに隔離しstable migrationを発生させない。
- 意味/組織: actor ID差と「独立した心」を別フィールド・findingとして明示する。

## 4. 候補と反実仮想

| Candidate | 局所利益 | 外部化コスト | Severity | Confidence | 判定 |
|---|---|---|---:|---|---|
| runtimeのmodel/context/session申告でindependenceを満たす | policy判定が容易 | 虚偽・相関をacceptance consumerへ転嫁 | 11 (E3/A2/F2/K3/T1) | C2 | externalization、除去済み |
| policyがcore review trust ruleを再定義 | 一箇所で強制可能 | #40との意味分岐、既存spaceの互換性破壊 | 12 (E3/A3/F2/K3/T1) | C2 | inversion、除去済み |
| anchorをaccepted evidenceとして返す | 呼出側が簡単 | evidence review/gateを迂回 | 12 (E3/A3/F2/K3/T1) | C2 | externalization、除去済み |

反実仮想Aはruntime申告をそのまま満足条件にする。Bはすべてを「観測不能」としてpolicy機能を持たない。Cはledger-verifiable/runtime-attested/not-observableを分離し、anchorの決定的整合だけを返して通常review seamで止める。Cを採用した。局所APIは少し増えるが、runtime・acceptance・組織境界を広げると優位性が反転する。

## 5. 残存不確実性

実actor/capability recordへのadapter、複数anchor provider、運用上のfalse-positive率は未観測（C1）。次の証拠は#48 ingestとの統合fixtureと、CaseGraphen canonical reviewからのlineage projectionである。自動accepted mutationを追加しないことを介入条件とする。

独立レビューで、配列要素をそのままquorum人数として数えると同じreport IDの再送が別verifierを装える局所最適も検出した（E3/A2/F2/K2/T1=10、C2）。A: 配列件数、B: caller側dedupe、C: policy reconcilerでreport identityを一度だけ数える、のCを採用し、duplicate/empty identityをledger-verifiable違反としてfail closedにした。

横断監査では、report IDだけを変えた同一actorが複数quorum席を占めることと、同じanchor IDの矛盾観測がmap上書きで入力順依存になることを追加検出した（E3/A3/F3/K3/T1=13、C3）。actor identityとanchor identityも一意性をpolicy境界で検査し、衝突時はquorum/anchor双方をfail closedにした。
