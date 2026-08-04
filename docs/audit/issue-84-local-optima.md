# Issue #84 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: deployment bundle生成、永続化後の検証、reviewed deployment authority、resource allocatorへのauthority受け渡し。
- 主要な結論: artifact単体のcontent-address整合性をbundle provenanceとみなす局所最適を解消した。authority境界では保持したcanonical inputから再compileし、全artifact bytesとmanifestの一致を要求する。
- 高確度候補数: 修正済み1、未解消0。
- 証拠上の制約: remote filesystem破損と旧compiler versionからのmigrationは未pilot。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- accepted reviewが承認したtopology/policyだけから生成されたdeploymentへ、resource/runtime authorityを限定する。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` 評価境界 | artifact/manifest hash | compiler入力、全出力、host、allocator |
| `M` 評価指標 | byte integrity | deterministic compiler provenance |
| `N` 変更可能範囲 | verifier | compiler contract、schema、host委譲 |
| `T` 時間軸 | bundle読込1回 | 永続化、差替え、compiler version更新 |

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `src/graph_compiler.rs` | compile→verify→authority | Rust library境界 |
| 実行 | graph compiler unit/property tests | 全artifactの再address差替え | local process |
| 進化 | ADR 0023、issue #84 | review authorityの拡張 | v0のみ |
| 意味・組織 | MCP host guide | artifact writerとauthority owner | deployment運用はpilot前 |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | hash自己整合性をcompiler provenanceと同一視 | verifierが軽量 | 任意artifact writerが未生成bundleを構成可能 | reviewed authority | 12 | C3 | 修正済み |

## 5. 上位候補の詳細

### 5.1 識別

- Candidate ID: I84-C1
- 名称: Self-consistency-only deployment verification
- 対象実装: `verify_deployment_bundle`
- 所有モジュール / サービス: graph compiler / operational host
- 所有チーム: CaseGraphen
- 導入時期: reviewed deployment authority導入時
- 調査者: Codex

### 5.2 事実・推論・仮説

- [Evidence] `src/graph_compiler.rs` は全bundleに`compiler.inputs.json`を生成し、verifierが内部でrequestを復元して`compile_execution_topology`を再実行する。
- [Evidence] verifierは再生成manifestとpath→hash/bytes mapの完全一致後にだけprivate-field `VerifiedDeploymentBundle`を返す。
- [Evidence] governed deployment-bundle schemaは12のcanonical artifact path（retained inputsとcase mappingを含む）を必須・一意にし、空manifest exampleを許さない。
- [Evidence] substitution testは各artifactへbyte変更を加えてmanifest/hashも再計算し、`deployment_bundle_semantic_mismatch`を要求する。
- [Inference] public callerがhash整合したartifact directoryを構成してもcanonical compiler由来でなければauthorityへ進めない。
- [Hypothesis] 大規模topologyで再compile costがresource reservation latencyを支配する可能性がある。

### 5.3 局所的合理性

- 局所目的: persisted bytesの改竄検出。
- 局所指標: hash/length一致。
- 直接の受益者: bundle loader。
- 現在得られている利益: 軽量なI/O検査。
- 導入時の制約: compiler出力をopaque proofへ接続する必要があった。
- 現在も有効な制約: artifact directoryはuntrusted。
- 失効した制約: verifierはcompiler semanticsを再利用できない、という制約は存在しない。

### 5.4 評価条件

- `B`: loaderからreview/allocator authorityまで。
- `M`: integrityではなくprovenance。
- `N`: retained inputsとdeterministic compilerを変更可能。
- `T`: bundle作成からresource releaseまで。

### 5.5 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| hash一致のみ検査 | semantic substitution | host/operatorが出所を暗黙に信頼 | 運用者 | reservationごと | issue #84 review |

### 5.6 四観測面の証拠

- 構造: verifierとcompilerは同一moduleにあり、rule複製なしで再compile可能。
- 実行: 全artifact substitution matrixでsemantic refusalを観測する。
- 進化: compiler versionをretained inputへ固定し、異なるversionを拒否する。
- 意味・組織: bearer認証やartifact書込権限をdeployment authorityと混同しない。

### 5.7 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | hash検査が速い | semantics不明 | 再compileで意味を検査 | CPU増 | 条件付き |
| モジュール | 小さいverifier | compilerとの対応なし | canonical rule再利用 | input artifact追加 | 再compile |
| 機能 | loader成功率 | forged bundle許容 | authority chain維持 | strict refusal | 再compile |
| システム | I/O低減 | resource authority誤付与 | end-to-end provenance | latency | 再compile |
| 運用・組織 | 手順が単純 | operator信頼へ外部化 | writer/authority分離 | version管理 | 再compile |
| ライフサイクル | 現versionのみ | drift検出不能 | version固定 | migration必要 | 再compile |

- 反転する最小境界: reviewed authority constructor。
- 反転する指標: byte integrityからcompiler provenance。
- 反転する時間軸: bundleがuntrusted storageから再読込された時点。

### 5.8 反実仮想

#### A. 現状維持

- 定常コスト: artifact writerをauthority principalとして信頼。
- 将来コスト: artifact追加ごとに手動cross-check。
- リスク: self-consistent forged deployment。

#### B. 最小限の局所改善

- 変更: 各artifactをtyped parseして個別cross-check。
- 利益: 明白な不一致を検出。
- 残る問題: compiler lowering ruleの二重実装。
- 移行コスト: artifactごとのvalidator維持。

#### C. 境界をまたぐ構造変更

- 変更: canonical input保持とdeterministic recompile（採用）。
- 成立条件: compilerがpure/deterministicであること。
- 定常利益: 新artifactもcompiler output equalityへ自動包含。
- 新たなコスト: verify時のCPU、version固定。
- 移行の谷: 旧bundleはinputs artifact不足で拒否。
- ロールバック: experimental v0の旧verifierへ戻せるがauthority強度は低下。

### 5.9 スコア

- `E` 3、`A` 2、`F` 3、`K` 2、`T` 2。
- `Severity`: 12/15。
- `Confidence`: C3。

### 5.10 判定

- 分類: `externalization`（修正済み）。
- 判定理由: loader局所の軽量性がauthority境界でprovenance欠落へ反転した。
- 反証となり得る情報: artifact storageへの書込がcanonical compiler processだけに暗号学的に限定される証拠。
- 未検証事項: 大規模bundleの再compile latency。
- 次に取得すべき証拠: scale pilotでverify時間を記録。

## 6. 横断的な補償構造

- 共通する変換・例外・再試行: 個別artifact validatorを増やさずcanonical compilerへ集約した。
- 所有権・KPIに起因する再発構造: loader成功率だけを指標にするとauthority provenanceが再び境界外化する。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| verify時の再compile | 重複計算 | authority境界だけで実行しruleは複製しない | security costとして合理的 |
| retained reviewed fields | caller-constructibleに見える | deserialization単独ではproofを公開せず、全出力再現にのみ使用 | authority-free evidenceとして合理的 |

## 8. 未検証事項

- 旧bundle migration、remote filesystem、非常に大きいpolicy集合での性能。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | verify latency distribution | 再compile運用コスト | #85 scale pilotへ計測追加 |
| 2 | old-version fixture | migration refusalの安定性 | version mutation test |

## 10. 介入判断の前提

- 変更可能な範囲: experimental compiler/schema/host。
- 移行期間: v0のため破壊変更可。
- 一時悪化可能: reservation latency。
- 制約: ledger authorityをartifact writerへ移さない。
- ロールバック要件: compiler input artifactを含むbundleを保持し再検証可能にする。
