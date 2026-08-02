# Issue #54 実装局所最適監査

## 1. エグゼクティブサマリー

- 調査範囲: ExpansionPolicy、all-seen dedupe、round/cost/node termination、proposal/review transition、topology hash binding
- 主要な結論: discoveryはaccepted graphを直接変更せず、content-addressed unreviewed proposalでreview seamへ停止する。監査でcandidate単位のruntime-reported costだけを数えるとduplicate discoveryの実行costがbudget外になる局所最適を発見し、controller境界でaccounted round costを必ず計上する形へ修正した。review後topologyもidentityとreal graph lintで検査する。
- 高確度候補数: 修正済み2件。残る弱い候補1件。
- 証拠上の制約: fixture実行のみ。cost anchor、長時間探索、review authority、永続化の運用証拠はない。

## 2. システム成果と評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` | 一候補/一round | discovery runtime、review、次topology revision、長時間fleet |
| `M` | dedupe、proposal生成 | 総cost、収束、memory、trust、accepted graph integrity |
| `N` | controller | policy/schema/topology linter/reviewer callerを同時変更可能 |
| `T` | 一attempt | 複数round、duplicate反復、review後の次attempt |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/dynamic_expansion.rs`, policy schema | state、limits、hash、review | 静的 |
| 実行 | `tests/dynamic_expansion.rs` | all-seen、dry、limits、cost、hash switch、review | fixture |
| 進化 | shared diff | 今回のみ | historyなし |
| 意味・組織 | acceptance-kernel境界 | runtime proposes/reviewer decides | authority実装なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | unique candidateのreported costだけを計上 | candidate処理が自己完結 | duplicate/rejected探索costがbudget外 | candidate→round/runtime | 11 | C2 | `externalization`、修正済み |
| 2 | review APIが任意topology hashを返す | generic patch適用器が不要 | invalid/別space topologyを次revisionへ渡せる | proposal→reviewed deployment | 10 | C2 | `externalization`、修正済み |
| 3 | all-seen集合をmemory保持 | 正確で単純なdedupe | 大量rejected candidateでmemory増加 | fixture→long-running fleet | 7 | C1 | `time-delayed`候補 |

## 5. 上位候補カード

### LO-54-1: candidate-local cost accounting

- [Evidence] duplicateはdedupe後に処理終了するためcandidate付随costだけでは二回目以降を計上できない。
- [Evidence] 修正後testは同一candidateを二round報告し、二round分のaccounted costで`max_cost` haltとfindingを確認する。
- 局所的合理性: candidateにcostを置けばAPIが小さく、accepted candidateの価格を説明しやすい。
- 外部化: discoveryを実行したruntime/operatorがduplicate探索costを負い、policy counterには現れない。

| 境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| candidate | 自己完結 | duplicateは0扱い | round costは別入力 | caller責務 | 現在案 |
| runtime round | 小API | 実行cost欠落 | duplicate含め全cost計上 | accounting source必要 | 代替案 |
| fleet | proposal単価明確 | budget bypass | hard stopが実作業へ対応 | anchor運用 | 代替案 |

- A 現状維持: candidate reported costのみ。
- B 最小改善（採用）: `process_round`へfinite/non-negativeな`accounted_round_cost`を必須入力とし、dedupe前のround全体を計上する。
- C 構造変更: metered runtime/anchorからcost ledgerを導入する。P1/P2 control planeが必要なため保留。
- `E=3,A=2,F=2,K=3,T=1`, Severity `11`, Confidence `C2`, `externalization`。

### LO-54-2: review済みhashの過小検証

- [Evidence] proposal patchはgeneric JSONなのでcontroller自身はapplyできない。単にcaller提供topologyをhashするとproposalと無関係なinvalid topologyもtransitionにできる。
- [Evidence] 修正はtopology/case-space identityを固定し、#45のreal linterによるdeterministic errorを拒否し、baseと同一hashも拒否する。accepted graphは依然mutateしない。
- A 任意hash、B identity+lint+distinct hash（採用）、C typed patch compilerでproposalとの完全対応を証明。Cは#49 compilerとの語彙統合が必要。
- `E=2,A=2,F=3,K=2,T=1`, Severity `10`, Confidence `C2`, `externalization`。

## 6. 補償ハロー

- controllerはruntime/scheduler/model callerを持たず、review authorityも自己発行しない。
- all-seenにはacceptedだけでなくrejected/deferred/duplicate元を含め、`confirmed`だけのdedupeによる再探索を防ぐ。
- max iteration/node/cost/dry haltはtyped findingを伴い、silent terminationを避ける。

## 7. 候補ではなかったもの

| 対象 | シグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| rejected/deferredもall-seen | 再検討不能 | policyの明示目的が再発見loop防止 | reviewで新policy/attemptを作る |
| review transitionがaccepted graphをmutateしない | 一工程増える | acceptance kernelの非交渉境界 | proposal≠acceptance |
| attempt中hash固定 | streaming改善を妨げる | logical graphの混在を防止 | 新hashは次attempt |

## 8. 未検証事項

| 優先度 | 証拠 | 不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | accounted costのanchor/attestation | caller値の信頼性 | control-plane integration test |
| 2 | 10万candidateのmemory/latency | all-seenの反転点 | benchmark |
| 3 | reviewed patchとcompiled topologyの対応 | arbitrary reviewer edits | #49 compilerとのproperty test |

## 9. 介入判断の前提

- `accounted_round_cost`をruntime自己申告だけでverified costと呼ばない。
- all-seenを永続化する場合もtopology hash/policy ID/attempt IDをkeyにし、別graphへ流用しない。
- dynamic resultからaccepted graphを自動更新しない。reviewed transition後も既存ledger gateを通す。
## 横断監査後の補正

初回監査がcost/node/iterationだけを「hard limits」として最適化し、要求されたlatency境界、candidate disposition契約、provenanceをpolicy外へ押し出していた局所最適を検出した（E3/A2/F3/K3/T1=12、C3、`externalization`）。A: runtime任せ、B: metadataとして任意記録、C: policy schema/Rust型/controller haltへ統合、を比較してCを採用した。`max_latency_ms`はaccounted round latencyを全candidate（duplicate/rejectを含む）について累積し、到達時は`MaxLatency`でdeferする。candidate dispositionは`unreviewed_morphism_proposal`に固定し、policy provenanceも必須化した。

同時に、`review_accepted`という関数名だけでreviewの存在を仮定していた局所最適を補正した。新しいtopology hashへのtransition recordはcanonical review由来のaccepted review IDとrevision IDを必須にし、accepted graph mutation APIは依然存在しない。

最終trust監査で、このbinding型自体のpublic fieldをcallerが任意生成できるため、非空文字列検査ではreviewを偽装できる重大欠陥が残っていた（E3/A3/F3/K3/T1=13、C3）。型の全fieldをprivate化し、唯一のconstructorはGraph Compilerがcanonical CaseGraphen review logから生成するopaque `CompilationMode::Reviewed`を要求するよう変更した。さらにreviewed claim metadataの`expansion_proposal_id`、proposal ID、reviewed topology hashを同時に一致させる。proposal modeや別proposal/hashはbindingを生成できない。
