# 0033-bug-fix-delayed-frame-sequence-change

- Created: 2026-08-17
- Completed: 2026-08-19
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

NVIDIA の公開仕様では EOS による display 待ち picture の排出は説明されているが、sequence 変更時に同じ排出が必ず完了するとは明記されていない。

したがって、この項目は develop ブランチの確定した不具合ではなく、実機検証が必要なリスクとして扱う。

## 調査結果

調査用ブランチ `feature/fix-delayed-frame-sequence-change` は develop へマージしない。テストデータ・テストコード・スキル・README の追記は残らない。検証の範囲と結論は本 issue に集約する。

2026-08-19 の実機検証 (GitHub Actions `Test (NVIDIA GPU)`) では、次の条件だけを確認した。

- 現行の destroy + create 経路 (`DecoderState::handle_video_sequence` が既存 decoder を破棄してからジオメトリを更新する)
- `max_display_delay = 2`
- B フレームを含む H.264 / H.265 の解像度変化ストリーム
- 入力は 1 アクセスユニットずつ `Decoder::decode` に渡す

入力ストリームの構成は次のとおり。既存の `testdata/resolution-change/` (issue 0031 / PR #22 で develop に入っている B フレームなしデータ) と同じ解像度遷移で、B フレームあり版を調査ブランチ上に置いて使った。

- 45 フレーム、3 セグメント: 320x240 x15 + 256x160 x15 + 320x240 x15
- 各セグメント先頭はキーフレームで、パラメータセット (SPS / PPS / VPS 等) を含む
- 調査ブランチ上のファイル名: `h264_bframes.h264` / `h265_bframes.h265`

確認できた出力は次のとおり。

- 全フレームが出力される (フレーム欠落なし)
- エラーは通知されない
- 寸法の枚数は 320x240 x30 + 256x160 x15
- 寸法遷移はちょうど 2 回で、320x240 → 256x160 → 320x240 の順

この結果が支持するのは、「現行の destroy + create 経路では、上記の入力と `max_display_delay = 2` において、利用者に見える新ジオメトリ誤適用は起きない」ことまでである。

次は確認していない。

- `pfnSequenceCallback` と `pfnDisplayPicture` の実順序 (寸法と枚数からの推論であり、待ち picture が残らないことと、残っても sequence より前に排出されることの区別はしていない)
- Y / UV の stride、既知座標の画素値、UV プレーンへの混入
- `max_display_delay` が 2 以外の値
- decoder を破棄せずジオメトリだけ更新する経路 (reconfigure)

解像度変更点のキーフレームは DPB 参照を切る。一方 `max_display_delay` は parser の display 遅延キューであり、キーフレームだけではキューが空になる保証はない。今回の合格は「境界をまたぐ待ちがなかった」証明ではなく、「現行経路では誤適用として観測されなかった」証明である。

現行経路では `handle_video_sequence` が decoder を破棄する。待ち display が sequence の後に来れば map / copy 失敗や欠落でテストが落ちやすい。合格は現行経路の判断材料にはなるが、decoder を残す経路へは一般化しない。

## 設計方針

最初から picture ごとの状態管理が必要だとは決めない。

まず、sequence callback の前に旧 sequence の display 待ち picture が排出されるかを NVIDIA GPU 実機で検証する。

旧 sequence の picture が残らない場合は、その前提と検証結果を記録する。本 issue では調査ブランチをマージしないため、記録先はコードコメントやテストではなく本 issue とする。

旧 sequence の picture が残る場合は、sequence callback で更新した最新値を、すべての display callback に無条件で使用しない。次のいずれかを実機で検証して採用する。

1. `CUVIDPICPARAMS.CurrPicIdx` と `CUVIDPARSERDISPINFO.picture_index` を利用して picture ごとのジオメトリを関連付ける
2. sequence 変更前の display 待ち picture を完了させてから状態を更新する

picture index を利用する場合は、次を管理する。

- 表示幅と表示高さ
- mapped output surface の高さ
- display area
- decoder session の世代
- picture の表示完了後に情報を破棄するライフサイクル

`CUVIDPICPARAMS.CurrPicIdx` は実環境では bindgen が `third_party/nvcodec/include/cuviddec.h` から生成して利用できる。ただし `build.rs` の docs.rs 用スタブは `CUVIDPICPARAMS` を opaque にしているため、docs.rs ビルドで型を公開する場合はスタブ拡張が必要になる。

調査結果を踏まえた採用判断は次のとおり。

- 現行の destroy + create 経路では、方式 1 と方式 2 を実装しない
- これは「今回の条件では利用者に見える誤適用が観測されなかった」ためであり、「NVIDIA parser が sequence 変更時に必ず排出する」ことの証明ではない
- decoder を残す reconfigure 経路での待ち picture と新ジオメトリの混在は、issue 0024 の公開設定判断チェックポイントで再検証する

## テスト戦略

モックやスタブは使わず、NVIDIA GPU 実機で検証する。

`max_display_delay > 0` かつ B フレームを含む H.264 または H.265 の入力を使用する。

各出力フレームについて次を確認する、としていた。

- フレーム順序とフレーム数
- `width()` / `height()`
- Y / UV の stride とプレーンサイズ
- 既知の座標にある Y / U / V の値
- sequence callback と decode / display callback の順序
- sequence 変更境界で、以前の sequence の画素や Y データが UV プレーンへ混入していないこと

実機で実施したのは、フレーム数、エラーの有無、`width()` / `height()` の枚数と遷移順序までである。stride・画素値・callback 順は測っていない。追加の実機確認は、現行経路の修正可否を決めるには必須ではないと判断し、本 issue では行わない。

## 完了条件

- `max_display_delay > 0` と B フレームを含む sequence 変更について、現行経路での利用者に見える誤適用の有無が実機で確認されている
  - [達成] `max_display_delay = 2` の H.264 / H.265 で、欠落なし・寸法枚数・寸法遷移が正しいことを `Test (NVIDIA GPU)` で確認した。callback 順そのものは未計測
- 旧 sequence の display 待ち picture が残る場合は、display callback がその picture と無関係な最新ジオメトリを使用しない
  - [該当なし] 現行経路では誤適用が観測されなかったため、修正はしない
- 旧 sequence の picture が残らない場合、または誤適用が観測されない場合は、その前提と検証範囲が記録されている
  - [達成] 本 issue の「調査結果」に記録した。調査ブランチはマージしないため、コードコメント・テスト・README・スキルへは残さない
- 必要に応じて修正し、`CHANGES.md` の `develop` セクションに `[FIX]` エントリが追加されている
  - [該当なし] 修正を行っていないため `[FIX]` エントリは追加しない

## 解決方法

現行の destroy + create 経路ではジオメトリ管理を変更しない。調査ブランチ上のテストと B フレームデータはマージしない。検証内容は本 issue に残し、スキル (`skills/shiguredo-nvcodec/SKILL.md`) と `testdata/resolution-change/README.md` は更新しない。

### 変更対象ファイル

- `src/decode.rs` — 遅延フレームの検証に応じたジオメトリ管理、実機テストを修正する
  - [実施しない] 現行経路の修正は不要。調査ブランチのテスト追加もマージしない
- `testdata/resolution-change/` — B フレームを含む入力を追加する
  - [実施しない] 調査ブランチ上でのみ使用し、develop には残さない。ストリーム仕様は「調査結果」に記載した
- `build.rs` — 方式 1 を採用する場合は docs.rs 用スタブに `CUVIDPICPARAMS` の実フィールドを追加する
  - [不要] 方式 1 を現行経路では採用しない
- `skills/shiguredo-nvcodec/SKILL.md` — 遅延フレームの検証結果に応じた説明を更新する
  - [不要] 調査ブランチをマージしないため、記録先は本 issue とする
- `CHANGES.md` — 修正を行った場合は `develop` セクションに `[FIX]` エントリを追加する
  - [不要] 修正を行っていないため変更しない

## 関連 issue

- 0024 (open): reconfigure と decoder 再作成のハイブリッド化。公開設定の判断チェックポイントに `max_display_delay > 0` や B フレームを含む入力での遅延フレーム混在の確認がある。本 issue の結論は destroy + create に限定し、reconfigure 時の再検証は 0024 に残す
- 0031 (closed): display area の非ゼロ原点による `DecodedFrame` の寸法と画素領域の不一致を修正する。本 issue は、そこから分離した遅延フレームの検証と条件付き修正を扱う。`testdata/resolution-change/` の B フレームなしデータは 0031 側の基盤を使用する
