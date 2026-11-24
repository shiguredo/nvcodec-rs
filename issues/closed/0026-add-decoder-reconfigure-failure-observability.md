# 0026-add-decoder-reconfigure-failure-observability

- Created: 2026-08-06
- Completed: 2026-08-07
- Branch: feature/add-decoder-reconfigure-failure-observability

**本 issue は 0027 (Decoder / Encoder 統計値 API を追加する) に吸収されました。** `Decoder::reconfigure_failure_count()` の個別 API ではなく、0027 の統一 API (`Decoder::stats() -> DecoderStats`) の一項目として `reconfigure_failure_count` を提供する方針に変更しています。`Error::function()` の pub 化については 0027 では扱わないため、必要になった段階で別 issue を起票します。

## 目的

`cuvidReconfigureDecoder` の失敗を利用側から堅く識別・集計できるようにする。0024 で導入した reconfigure 経路 (`max_coded_width` / `max_coded_height` 両方 `Some`) のフォローアップ。

利用側が「reconfigure が失敗したら Decoder を作り直す」フォールバックを自前実装できる材料を揃える。自動フォールバックは行わない (silent な挙動変化を避け、フォールバックポリシーは利用側の主権とする)。

## 現状

0024 で reconfigure 経路を追加した。`cuvidReconfigureDecoder` が失敗すると `handle_video_sequence_inner` (`src/decode.rs`) が `Err` を返し、`handle_video_sequence` 経由で `frame_tx.send(Err(...))` として利用者に通知される。エラーは `Error` 型 (`src/error.rs`) で、`function: &'static str` フィールドに `"cuvidReconfigureDecoder"` が入るため意味的な識別はできる。

ただし以下 2 点が不足している。

- **`Error::function` に公開アクセサがない**: `Error` 構造体の `function` フィールドは非 pub で、`impl Error` にも accessor が定義されていない (`src/error.rs`)。利用側から reconfigure 起因かを識別するには `Debug` / `Display` 出力の文字列パースに頼るしかなく脆い
- **reconfigure 失敗の集計 API がない**: 「これまでに何回失敗したか」を取れる API がなく、運用側で問題を早期検知するメトリクスが組みづらい

利用側で自前フォールバックする場合の想定手順:

1. handler / `decode()` 戻り値でエラー受信
2. `Error` から reconfigure 起因かを識別
3. `Decoder` を drop
4. 新 `Decoder` を作成する (方針次第で `max_coded_width = None` にして destroy+create モードにする等)
5. 以降の packet をそのまま feed し続ける (parser が次のシーケンスヘッダから pickup する)

現状は (2) が文字列パース依存で脆い。(1) の集計だけ欲しいユースケース (メトリクス収集) も現状はサポートしていない。

## 設計方針

以下 2 点で観測性を上げる。

### 1. `Error::function` の公開アクセサを追加する

`Error::function(&self) -> &str` (仮称) を pub で追加する。エラー発生元の関数名 (例: `"cuvidReconfigureDecoder"`) を利用側で取得可能にする。

現状 `function` フィールドは `&'static str` だが、pub API では `&str` として公開する (将来的に格納方式を変えても互換維持しやすいため)。

### 2. reconfigure 失敗カウンター API を追加する

`Decoder::reconfigure_failure_count(&self) -> u64` (仮称) を追加する。これまでに `cuvidReconfigureDecoder` が失敗した通算回数を返す。

- カウントは `DecoderState` (`src/decode.rs`) の `u64` フィールドで保持する
- `handle_video_sequence_inner` の reconfigure 分岐で `cuvid_reconfigure_decoder` が `Err` を返したときにインクリメントする
- 取得 API はワーカスレッドと非同期に呼ばれる可能性があるため、`AtomicU64` で保持するか、既存の `Job` チャネル経由で照会するかを検討する
  - 0024 で追加した `#[cfg(test)]` のカウンター (`create_decoder_count` / `reconfigure_decoder_count`) は `Job::QueryCallCounts` 経由で取得する仕組みになっている。本 issue のカウンターも同じ機構に乗せる案が既存パターンとしては整合する
  - 一方、`Decoder::reconfigure_failure_count()` は本番用途 (メトリクス収集) で頻繁に呼ばれる可能性があるため、`AtomicU64` の直接読みの方が軽量。実装時に選択する

自動フォールバックは意図的にスコープ外とする。silent な挙動変化を避け、フォールバックポリシー (destroy+create モードに切り替えるか、Decoder を丸ごと作り直すか、エラー扱いにするか) は利用側で決められるようにする。

## 完了条件

- `Error::function()` が pub で追加され、`Error.function` の値を文字列パースなしで取得できる
- `Decoder::reconfigure_failure_count()` (仮称) が追加され、reconfigure 失敗の通算回数が取得できる
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` の動的解像度変更節に「利用側で reconfigure 失敗をハンドリングする方法」のレシピを追記する (エラー識別 → Decoder 再作成 → packet 継続 feed の 5 ステップ)
- `CHANGES.md` に `[ADD]` エントリが追加されている
- カウンター取得 API の単体テスト or 結合テストがある (0024 で追加した `#[cfg(test)]` カウンターと重複・混同しない設計にする)

## 解決方法

### 変更対象ファイル

- `src/error.rs` — `impl Error` に `function()` の pub accessor を追加
- `src/decode.rs` — `DecoderState` に reconfigure 失敗カウンターフィールドを追加、`handle_video_sequence_inner` の reconfigure 分岐で失敗時にインクリメント、`Decoder::reconfigure_failure_count()` を追加
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` — 動的解像度変更節に fallback レシピと `reconfigure_failure_count()` の使い方を追記
- `CHANGES.md` — 追記例:
  - `- [ADD] reconfigure 失敗の観測性を追加する (Error::function() 公開 + Decoder::reconfigure_failure_count())`
  - `  - @担当者`

## 関連 issue

- 0024: 本 issue の前提となる `cuvidReconfigureDecoder` の導入
