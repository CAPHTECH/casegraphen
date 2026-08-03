# Issue #66 実装局所最適監査レポート

## 1. エグゼクティブサマリー

- 調査範囲: Rust 1.80 MSRV/toolchain 宣言、Quality workflow、release gate、Clippy、exit-code property tests、runtime digest。
- 主要な結論: 新しい local toolchain だけで変更を検証することは開発者個人には速いが、pinned CI の判定を外部化していた。GitHub Quality の実 failure と Rust 1.80 Clippy の再現を根拠に、version-visible gate、宣言間 drift assertion、MSRV-safe test oracle、共通 digest implementation へ修正した。
- 高確度候補数: 2 件（いずれも修正済み）。Rust 1.80 を維持する判断自体は現状の公開 MSRV と一致し、局所最適ではない。
- 証拠上の制約: fix commit の GitHub Actions は push 前なので未観測。local full gate は dirty-worktree packaging mode であり、clean checkout の最終証拠は CI が担う。

## 2. システム成果と評価条件

### 最終的に良くしたい成果

- local と CI が同じ compiler/Clippy contract を観測し、release gate の green が clean checkout の publishability を意味すること。

### 現在の評価条件

| 変数 | 現在の条件 | 調査で広げた条件 |
|---|---|---|
| `B` | 開発者の active Rust | Cargo MSRV、toolchain file、全 workflow、packaging consumer |
| `M` | 新 toolchain で compile/test | declared MSRV で fmt/clippy/test/package が一致 |
| `N` | lint suppression/1 test の変更 | test oracle、hash helper、gate assertion、docs の同時変更 |
| `T` | 現在の checkout | CI、release、将来の toolchain update |

制約は Rust 1.80 公開 MSRV の維持、lint の有用性を抑制しないこと、release gate が唯一の Quality command であること。

## 3. 使用した証拠

| 観測面 | 情報源 | 範囲 | 制約 |
|---|---|---|---|
| 構造 | `Cargo.toml:5`, `rust-toolchain.toml`, `.github/workflows/quality.yml:12`, `scripts/static-analysis.sh:21-60` | 3 宣言と executable assertion/version report | workflow pin は意図的重複 |
| 実行 | GitHub run `30770626429` | unknown lint、ExitCode equality、format_collect failure | fix 前の CI |
| 実行 | `rustup run 1.80.0 cargo clippy --all-targets --locked -- -D warnings` | fix 後 green | local macOS、clean checkout ではない |
| 実行 | `rustup run 1.80.0 sh scripts/static-analysis.sh` | fmt、install、Skill、Clippy、全 tests、package、stable/experimental schema inventory がすべて pass | dirty-worktree package mode |
| 進化 | Quality の連続 failure と issue #66 | local/CI semantic drift | 長期 release metrics なし |
| 意味・組織 | README toolchain contract | contributor が使用すべき version を可視化 | 利用者 feedback なし |

## 4. 候補一覧

| Rank | Candidate | Local benefit | Externalized cost | Inversion boundary | Severity | Confidence | Verdict |
|---:|---|---|---|---|---:|---|---|
| 1 | active 1.95 だけでの local validation | 最新 API/lint、短い feedback | CI/release owner が 1.80 failure を負担 | push/clean CI | 12 | C3 | `externalization`、修正済み |
| 2 | module/test ごとの hex digest formatting | call site だけで完結 | version-specific Clippy、allocation、hash rule 重複 | all-target Clippy | 6 | C2 | `time-delayed`、修正済み |
| 3 | workflow と toolchain file の version 二重記載 | action が確実に pin を install | update 時の drift | toolchain update | 5 | C2 | `time-delayed`、assertion で管理 |

## 5. 上位候補の詳細

### Candidate TOOLCHAIN-DRIFT-01

#### 事実・推論・仮説

- [Evidence] fix 前 GitHub Quality は Rust 1.80 で3種の failure を報告した。
- [Evidence] `src/cli.rs:231-240,307-327` は exit status の domain invariant を MSRV-independent `u8` oracle で検査し、production boundary だけで `ExitCode` へ変換する。
- [Evidence] `scripts/static-analysis.sh:21-48` は Cargo MSRV、toolchain pin、全 dtolnay workflow pin の一致を fail closed で確認し、active rustc/Clippy を出力する。
- [Inference] 旧 local green は package contract でなく、開発者環境という狭い境界だけを最適化していた。
- [Hypothesis] CI runner と macOS での platform 差は fix commit の GitHub run が通るまで残る。

#### 局所的合理性

- mise の active toolchain を使うと download がなく feedback が速い。
- 新 Clippy はより多い問題を発見する。しかし公開 `rust-version = 1.80` の互換性証拠にはならない。

#### 補償ハロー

| 原因となる局所判断 | 境界外の影響 | 補償手段 | 負担者 | 頻度・規模 | 証拠 |
|---|---|---|---|---|---|
| local active toolchain を暗黙利用 | CI だけ unknown lint/API error | push 後に修正 cycle | contributor/reviewer | Quality run ごと | failed GitHub run |
| workflow pin を別管理 | version update が片側だけ進む | 人手 review | release maintainer | toolchain update ごと | 3宣言の静的比較 |
| ExitCode 自体を property 比較 | MSRV で `PartialEq` 不在 | test を新 Rust だけで実行 | CI owner | test build ごと | 1.80 compile error |

#### 境界拡張と優位性反転

| 評価境界 | 現在案の利益 | 現在案のコスト | 代替案の利益 | 代替案のコスト | 優位性 |
|---|---|---|---|---|---|
| 関数 | `ExitCode` の直接 assert が直感的 | MSRV compile failure | numeric domain oracle | helper 1個 | 新案 |
| モジュール | local helper が独立 | hash formatting 重複 | canonical hash helper | module dependency | 新案 |
| 機能 | active compiler で速い | release contract 不明 | pin で同じ判定 | toolchain download | 新案 |
| システム | 新 lint を先取り | CI red | single declared gate | update coordination | 新案 |
| 運用・組織 | 個人設定が自由 | reviewer が CI retry を負担 | logs に version 可視 | 数行のlog | 新案 |
| ライフサイクル | 常に最新 | MSRV が事実上失効 | explicit MSRV update | deliberate migration | 新案 |

- 反転する最小境界: local checkout から pinned Quality へ広げた時点。
- `E=3, A=2, F=3, K=3, T=1`, Severity `12/15`, Confidence `C3`。
- 分類: `externalization`。実 CI failure と pinned local reproduction の両方がある。

### Candidate HASH-FORMAT-02

- 局所目的: bytes をその場で hex 化し、新 helper dependency を避ける。
- 観測: iterator 内 `format!` は 1.80 Clippy `format_collect` で release failure。runtime integration は `crate::native_hash::sha256_hex` へ統合し、binary/test-only formatting は単一 allocation の `LowerHex`/`write!` に変更した。
- 反実仮想: A lint allow は drift を隠す。B call-site fold は通るが hash decision を重複する。C library code は canonical helper を再利用し、外部 binary/test は dependency boundary 内の最小 formatterを使う。調査範囲では C が優位。
- `E=1, A=2, F=1, K=1, T=1`, Severity `6/15`, Confidence `C2`, `time-delayed`、修正済み。

## 6. 横断的な補償構造

- version drift をコメント/記憶で補償せず、release script が宣言一致を検査する。
- production exit mapping と test oracle を分離し、標準型の version-specific trait 実装へ invariant test を結合しない。
- version-specific lint allow を削除し、1.80 でも新 compiler でも意味が明確なコードへ変更した。

## 7. 候補ではなかったもの

| 対象 | 当初のシグナル | 棄却理由 | 合理性 |
|---|---|---|---|
| Rust 1.80 の維持 | 古い compiler の lock-in | Cargo が公開 MSRV として宣言済みで、今回上げる利用者根拠がない | compatibility contract |
| active override 時の warning | fail closed でない | deliberate newer-toolchain testing を許し、CI は action pin で固定。宣言間 drift は別途 hard fail | testing flexibility |

## 8. 未検証事項

- fix commit の GitHub Actions Quality 結果。
- clean checkout での packaging gate（local dirty package は standalone build まで pass）。

## 9. 次に取得すべき証拠

| 優先度 | 証拠 | 解消する不確実性 | 取得方法 |
|---:|---|---|---|
| 1 | GitHub Quality green | Linux/clean checkout/pin の一致 | fix commit push 後の required check |
| 2 | toolchain bump rehearsal | drift assertion の将来運用 | 3宣言の意図的同時変更 test |

## 10. 介入判断の前提

- MSRV bump は `Cargo.toml`、`rust-toolchain.toml`、全 workflow、README を同一 change で更新する。
- temporary dual-toolchain matrix は必要になった場合だけ追加し、現在の単一 release contract を曖昧にしない。
- rollback は gate assertion を外すことではなく、MSRV-safe implementation を個別に戻す。
