# 0033-bug-fix-delayed-frame-sequence-change

- Created: 2026-08-17
- Completed: {YYYY-MM-DD} (例: 2024-07-01)
- Branch: feature/fix-delayed-frame-sequence-change
- Polished: {YYYY-MM-DD} (例: 2024-07-15)

## 目的

遅延フレームを伴う sequence 変更で、旧 sequence の display 待ち picture に新しいジオメトリを誤って適用しないことを NVIDIA GPU 実機で検証し、必要な場合だけ修正する。

issue 0031 は display area が非ゼロの入力で `DecodedFrame` の寸法と画素領域を一致させることに絞るため、本 issue で遅延フレームの検証と条件付き修正を扱う。

## 現状

`DecoderState` は現在の `width` / `height` / `surface_width` / `surface_height` を 1 組だけ保持している。

`DecoderState::handle_video_sequence` は sequence callback のたびにこれらを更新し、`DecoderState::handle_picture_display` は表示対象の picture に対応する情報ではなく、`DecoderState` の最新値を使って次を計算している。

- `DecodedFrame` の `width` / `height`
- Y / UV プレーンのコピー量
- 出力サーフェス上の UV プレーン開始位置

`CUVIDPARSERDISPINFO` は `picture_index` を持つが、解像度や display area を持たない。

`max_display_delay > 0` や B フレームを含む入力では、sequence callback 後に以前の sequence の display callback が来た場合、最新のジオメトリを古い picture に適用する可能性がある。

その場合は、フレーム欠落、異なる sequence のジオメトリ適用、map / copy の失敗につながり得る。

ただし、現時点では sequence callback 後に旧 sequence の display callback が発生することを再現できていない。

NVIDIA の公開仕様では EOS による display 待ち picture の排出は説明されているが、sequence 変更時に同じ排出が必ず完了するとは明記されていない。

`testdata/resolution-change/` は develop ブランチに存在せず、`max_display_delay > 0` や B フレームを含む入力を実機で検証するテスト基盤も未整備のため、この可能性を検証できていない。

したがって、この項目は develop ブランチの確定した不具合ではなく、実機検証が必要なリスクとして扱う。

## 設計方針

最初から picture ごとの状態管理が必要だとは決めない。

まず、sequence callback の前に旧 sequence の display 待ち picture が排出されるかを NVIDIA GPU 実機で検証する。

旧 sequence の picture が残らない場合は、その前提と検証結果をコードコメントまたはテストに記録する。

旧 sequence の picture が残る場合は、sequence callback で更新した最新値を、すべての display callback に無条件で使用しない。

次のいずれかを実機で検証して採用する。

1. `CUVIDPICPARAMS.CurrPicIdx` と `CUVIDPARSERDISPINFO.picture_index` を利用して picture ごとのジオメトリを関連付ける
2. sequence 変更前の display 待ち picture を完了させてから状態を更新する

picture index を利用する場合は、次を管理する。

- 表示幅と表示高さ
- mapped output surface の高さ
- display area
- decoder session の世代
- picture の表示完了後に情報を破棄するライフサイクル

`CUVIDPICPARAMS.CurrPicIdx` は実環境では bindgen が `third_party/nvcodec/include/cuviddec.h` から生成して利用できる。ただし `build.rs` の docs.rs 用スタブは `CUVIDPICPARAMS` を opaque にしているため、docs.rs ビルドで型を公開する場合はスタブ拡張が必要になる。

## テスト戦略

モックやスタブは使わず、NVIDIA GPU 実機で検証する。

`max_display_delay > 0` かつ B フレームを含む H.264 または H.265 の入力を使用する。

各出力フレームについて次を確認する。

- フレーム順序とフレーム数
- `width()` / `height()`
- Y / UV の stride とプレーンサイズ
- 既知の座標にある Y / U / V の値
- sequence callback と decode / display callback の順序
- sequence 変更境界で、以前の sequence の画素や Y データが UV プレーンへ混入していないこと

## 完了条件

- `max_display_delay > 0` と B フレームを含む sequence 変更について、callback の順序と旧 sequence の display 待ち picture の有無が実機で確認されている
- 旧 sequence の display 待ち picture が残る場合は、display callback がその picture と無関係な最新ジオメトリを使用しない
- 旧 sequence の display 待ち picture が残らない場合は、その前提と検証結果がコードコメントまたはテストに記録されている
- 必要に応じて修正し、`CHANGES.md` の `develop` セクションに `[FIX]` エントリが追加されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — 遅延フレームの検証に応じたジオメトリ管理、実機テストを修正する
- `testdata/resolution-change/` — B フレームを含む入力を追加する
- `build.rs` — 方式 1 を採用する場合は docs.rs 用スタブに `CUVIDPICPARAMS` の実フィールドを追加する
- `skills/shiguredo-nvcodec/SKILL.md` — 遅延フレームの検証結果に応じた説明を更新する
- `CHANGES.md` — 修正を行った場合は `develop` セクションに `[FIX]` エントリを追加する

## 関連 issue

- 0024 (open): reconfigure と decoder 再作成のハイブリッド化。公開設定の判断チェックポイントに `max_display_delay > 0` や B フレームを含む入力での遅延フレーム混在の確認がある
- 0031 (open): display area の非ゼロ原点による `DecodedFrame` の寸法と画素領域の不一致を修正する。本 issue は、そこから分離した遅延フレームの検証と条件付き修正を扱う。`testdata/resolution-change/` は本 issue で構築された基盤を使用する
