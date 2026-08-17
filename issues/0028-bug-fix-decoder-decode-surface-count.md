# 0028-bug-fix-decoder-decode-surface-count

- Created: 2026-08-07
- Branch: feature/fix-decoder-decode-surface-count

## 目的

NVDEC parser が使用する DPB の surface 数と、decoder が確保する内部 decode surface 数を一致させる。

develop ブランチでは、`CUVIDEOFORMAT.min_num_decode_surfaces == 1` の場合に sequence callback の戻り値が parser の DPB 数を更新しないため、parser と decoder の surface 数が一致しない可能性がある。

この問題は `cuvidReconfigureDecoder` の導入前から存在するため、issue 0024 を待たずに修正する。

codec 別の固定値へ一律に引き上げる変更は行わない。

`min_num_decode_surfaces` を超える surface 数は性能と GPU メモリ使用量の調整であり、正しさのための必須条件ではない。

## 根拠

### `min_num_decode_surfaces` の意味

Video Codec SDK 13.0.19 に同梱している `third_party/nvcodec/include/nvcuvid.h` の `CUVIDEOFORMAT.min_num_decode_surfaces` には、次の意味が記載されている。

- 正しいデコードのために必要な最小 surface 数である
- `CUVIDDECODECREATEINFO.ulNumDecodeSurfaces` に使用できる
- この値を使用すれば正しい動作と最小の video memory 使用量が保証される
- より大きい値は性能と memory 使用量のバランスを実験して決める
- `min_num_decode_surfaces` より小さい値は使用できない

NVIDIA Video Decoder API Programming Guide 13.0 の「4.1.1. Creating a parser」も、parser が報告した `min_num_decode_surfaces` を `CUVIDDECODECREATEINFO.ulNumDecodeSurfaces` に使用し、parser の DPB 数と decoder の surface 数を一致させる手順を説明している。

- https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvdec-video-decoder-api-prog-guide/index.html#creating-a-parser

したがって、`min_num_decode_surfaces` では DPB が不足して正しくデコードできないという従来の前提は採用しない。

### sequence callback の戻り値

`third_party/nvcodec/include/nvcuvid.h` の `PFNVIDSEQUENCECALLBACK` では、戻り値を次のように定義している。

- `0`: 失敗
- `1`: 成功。ただし parser の `ulMaxNumDecodeSurfaces` は更新しない
- `2` 以上: 成功し、parser の `ulMaxNumDecodeSurfaces` を戻り値で上書きする

`min_num_decode_surfaces` を decoder と parser の両方へそのまま適用できるのは、値が 2 以上の場合だけである。

### reconfigure による surface 数の増加

従来の issue には、`cuvidReconfigureDecoder` では `ulNumDecodeSurfaces` を後から増やせないと記載していた。

しかし、Video Decoder API Programming Guide 13.0 の「4.5. Allocating decode surfaces dynamically」は、`CUVIDPICPARAMS.CurrPicIdx` が現在の decoder surface 数以上になった場合に、`cuvidReconfigureDecoder` で `CUVIDRECONFIGUREDECODERINFO.ulNumDecodeSurfaces` を増やす手順を説明している。

- https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvdec-video-decoder-api-prog-guide/index.html#allocating-decode-surfaces-dynamically

したがって、reconfigure に備えて初回 decoder 作成時から codec 別の固定値を確保する必要はない。

### NVIDIA 公式サンプルの位置付け

NVIDIA 公式サンプルの `NvDecoder::GetNumDecodeSurfaces` は、H.264 で 20、VP9 で 12、HEVC で解像度に応じた値、それ以外で 8 を返している。

- https://github.com/NVIDIA/video-sdk-samples/blob/master/Samples/NvCodec/NvDecoder/NvDecoder.cpp

これはサンプルが選択した性能上の余裕であり、API が正しさのために要求する codec 別固定値ではない。

従来の issue に記載していた H.264 / HEVC = 20、VP9 / AV1 = 12、VP8 = 8、JPEG = 1 という表は、NVIDIA 公式サンプルの実装とも一致しないため削除する。

## 現状

### develop ブランチで問題が確認できない経路

`src/decode.rs` の `handle_video_sequence_inner` は、次の 2 箇所に `format.min_num_decode_surfaces` を使用している。

- `CUVIDDECODECREATEINFO.ulNumDecodeSurfaces`
- sequence callback の戻り値

`min_num_decode_surfaces >= 2` の場合、sequence callback の戻り値によって parser の DPB 数が同じ値へ上書きされる。

decoder は sequence callback ごとに同じ値で再作成されるため、この条件では parser と decoder の surface 数が一致している。

H.264 / HEVC / VP8 / VP9 / AV1 の通常の入力について、codec 別固定値へ引き上げなければ正しくデコードできない問題は確認できていない。

issue 0024 の検証中に発生した `CUDA_ERROR_INVALID_VALUE` は、テストデータの解像度が hardware の最小デコード解像度を下回っていたことが原因であり、DPB 不足を再現したものではない。

### develop ブランチでコード上確認できる問題

`DecoderState::new_with_codec` は、`DecoderConfig.max_num_decode_surfaces` を parser 作成時の `CUVIDPARSERPARAMS.ulMaxNumDecodeSurfaces` に設定している。

一方、`handle_video_sequence_inner` は decoder を `format.min_num_decode_surfaces` で作成し、同じ値を sequence callback から返している。

`min_num_decode_surfaces == 1` の場合、sequence callback は成功を返すだけで parser の DPB 数を更新しない。

その結果、次の組み合わせが発生し得る。

- parser は `DecoderConfig.max_num_decode_surfaces`、または以前の sequence で設定された DPB 数を維持する
- decoder は 1 decode surface だけを確保する

`CUVIDPARSERPARAMS.ulMaxNumDecodeSurfaces` は parser が循環利用する picture index の範囲を決める。

parser が decoder の surface 数以上の `CUVIDPICPARAMS.CurrPicIdx` を通知した場合、`cuvidDecodePicture` が `CUDA_ERROR_INVALID_VALUE` で失敗する可能性がある。

JPEG は `min_num_decode_surfaces == 1` になり得るため、解像度変更や reconfigure を使用しない場合も影響対象になる。

コードと NVDEC の callback 契約の不一致は確認できるが、実際に範囲外の `CurrPicIdx` が通知されることとエラー内容は NVIDIA GPU 実機で確認する。

### `DecoderConfig.max_num_decode_surfaces` の契約

現在の rustdoc は `max_num_decode_surfaces` を「デコード用サーフェスの最大数」と説明している。

しかし、sequence callback が 2 以上を返すと parser の値が上書きされるため、現在の実装では利用側が指定した値を超える場合も、それより小さくなる場合もある。

本 issue では `max_num_decode_surfaces` を、正しいデコードに必要な最小値を許可するか判定する上限として扱う。

parser が報告した `min_num_decode_surfaces` が上限を超える場合は、利用側の resource 制約を無視せず、具体的なエラーを返す。

### issue 0024 の reconfigure 経路

reconfigure 導入後も、parser の DPB 数と次の値を一致させる必要がある。

- `CUVIDDECODECREATEINFO.ulNumDecodeSurfaces`
- `CUVIDRECONFIGUREDECODERINFO.ulNumDecodeSurfaces`
- sequence callback の戻り値

新しい sequence で必要な surface 数が増えた場合は `cuvidReconfigureDecoder` で増加させる。

reconfigure が失敗した場合の decoder 再作成と終端契約は issue 0024 で扱う。

この同期処理は reconfigure 固有ではないため、develop ブランチの decoder 再作成経路を先に修正し、issue 0024 は同じ決定処理を利用する。

## 影響範囲と優先度

| 対象 | 判定 | 対応 |
|---|---|---|
| develop ブランチで `min_num_decode_surfaces >= 2` の入力 | 問題を確認できていない | codec 別固定値へ引き上げず、リグレッションテストの対象とする |
| develop ブランチで `min_num_decode_surfaces == 1` の入力 | コード上確認できる問題 | issue 0024 を待たずに parser と decoder の surface 数を同期する |
| `DecoderConfig.max_num_decode_surfaces` より大きい最小 surface 数を要求する入力 | 公開設定と実装の不一致 | decoder を作成または破棄する前にエラーを返す |
| issue 0024 の reconfigure 経路 | develop の修正を引き継ぐ対象 | 同じ surface 数決定処理を create / reconfigure / callback で使用する |
| `min_num_decode_surfaces` を超える codec 別の余裕 | 性能最適化 | 本 issue の対象外とし、必要なら実測結果を伴う別 issue で扱う |

## 設計方針

### parser の初期 surface 数

`CUVIDPARSERPARAMS.ulMaxNumDecodeSurfaces` は、NVIDIA Video Decoder API Programming Guide 13.0 の「4.1.1. Creating a parser」に従い、sequence header を解析する前の仮値として 1 を設定する。

`DecoderConfig.max_num_decode_surfaces` は `DecoderState` に保持し、sequence callback で必要な surface 数が判明した後に上限として検証する。

`max_num_decode_surfaces == 0` は decoder 作成時の設定エラーとして拒否する。

### 実効 surface 数の決定

codec 別の固定値は使用しない。

parser と decoder へ適用する実効 surface 数は、次の規則で決定する。

1. `min_num_decode_surfaces == 0` は NVDEC からの不正な値としてエラーにする
2. `min_num_decode_surfaces > max_num_decode_surfaces` は利用側が指定した上限を満たせないためエラーにする
3. `min_num_decode_surfaces >= 2` なら、その値を使用する
4. `min_num_decode_surfaces == 1` かつ `max_num_decode_surfaces >= 2` なら、callback で parser の DPB 数を確実に更新できる最小値として 2 を使用する
5. `min_num_decode_surfaces == 1` かつ `max_num_decode_surfaces == 1` なら、parser の初期値と同じ 1 を使用する

この規則を private helper にまとめ、decoder 作成、sequence callback、reconfigure で共通利用する。

設定や `CUVIDEOFORMAT` の検証は、既存 decoder を破棄する前に行う。

### 実装コメント

将来の SDK 更新時に判断根拠を再検証できるように、次の実装箇所へ日本語コメントを残す。

- parser の `ulMaxNumDecodeSurfaces` を 1 で初期化する箇所
  - NVIDIA Video Decoder API Programming Guide 13.0「4.1.1. Creating a parser」
  - sequence callback の戻り値が 1 の場合は parser の DPB 数を更新しないこと
- 実効 surface 数を決定する helper
  - `CUVIDEOFORMAT.min_num_decode_surfaces` が正しいデコードに必要な最小値であること
  - `third_party/nvcodec/include/nvcuvid.h` の `CUVIDEOFORMAT` と `PFNVIDSEQUENCECALLBACK` が根拠であること
- reconfigure で surface 数を増やす箇所
  - NVIDIA Video Decoder API Programming Guide 13.0「4.5. Allocating decode surfaces dynamically」

コメントには issue 番号や issue ファイルへの参照を書かず、仕様名、節番号、制約そのものを書く。

## テスト戦略

モックやスタブは使用しない。

private helper の境界値は GPU を使わない単体テストで検証する。

- `min = 1`, `max = 1` では 1
- `min = 1`, `max = 2` 以上では 2
- `min = 2`, `max = 2` では 2
- `min = 8`, `max = 20` では 8
- `min = 9`, `max = 8` ではエラー
- `min = 0` または `max = 0` ではエラー

NVIDIA GPU 実機では少なくとも次を検証する。

- `max_num_decode_surfaces = 20` で JPEG をデコードできる
- `max_num_decode_surfaces = 1` で JPEG をデコードできる
- 各 decode callback の `CurrPicIdx` が decoder に設定した surface 数未満である
- H.264 / HEVC / VP8 / VP9 / AV1 の既存デコードにリグレッションがない
- `min_num_decode_surfaces` が異なる sequence へ切り替わった後もデコードを継続できる
- issue 0024 の実装後は、surface 数の増減を伴う reconfigure と decoder 再作成の両方で同じ条件を満たす

## 完了条件

- parser の `ulMaxNumDecodeSurfaces` が仮値 1 で初期化されている
- `DecoderState` が `DecoderConfig.max_num_decode_surfaces` を保持している
- `max_num_decode_surfaces == 0` が decoder 作成時に拒否される
- 実効 surface 数を決定する private helper が追加されている
- decoder 作成時の `ulNumDecodeSurfaces` と sequence callback の戻り値が同じ実効 surface 数を使用している
- `min_num_decode_surfaces == 1` の場合も parser と decoder の surface 数が一致する
- parser が要求する最小 surface 数が `max_num_decode_surfaces` を超える場合は、既存 decoder を破棄する前に具体的なエラーが返る
- issue 0024 の実装後は reconfigure の `ulNumDecodeSurfaces` も同じ実効 surface 数を使用する
- codec 別の固定 surface 数が追加されていない
- NVIDIA の仕様名、節番号、制約を説明する実装コメントが追加されている
- helper の境界値テストと NVIDIA GPU 実機テストが追加されている
- `DecoderConfig.max_num_decode_surfaces` の rustdoc と README が、上限の意味と上限不足時のエラーを説明している
- `CHANGES.md` の `develop` セクションに `[FIX]` エントリが追加されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — parser の仮 surface 数、上限の保持と検証、実効 surface 数の helper、create / callback / reconfigure の適用、単体テストと実機テストを追加する
- `README.md` — `max_num_decode_surfaces` の上限としての意味と、必要な最小値が上限を超えた場合のエラーを説明する
- `skills/shiguredo-nvcodec/SKILL.md` — decoder surface 数の決定規則と resource 制約を説明する
- `CHANGES.md` — 次の `[FIX]` エントリを追加する

```markdown
- [FIX] Decoder の parser DPB と内部 decode surface の数が一致しない場合がある問題を修正する
  - @sile
```

## 関連 issue

- 0024: reconfigure と decoder 再作成のハイブリッド化。本 issue の surface 数決定処理を reconfigure 経路でも利用する

## 旧方針を採用しない理由

issue 0024 の旧 feature ブランチでは、codec 別の固定値を採用する実装が commit `b54c66d` に含まれていた。

この変更は現在の issue 0024 のブランチには含まれていない。

当時観測した `CUDA_ERROR_INVALID_VALUE` は hardware の最小デコード解像度を下回るテストデータが原因であり、DPB 不足の再現事例ではなかった。

また、SDK 13.0 の仕様は `min_num_decode_surfaces` で正しいデコードが可能であることと、reconfigure によって surface 数を増やせることを明記している。

根拠のない codec 別固定値は、特に高解像度や多 decoder 構成で GPU メモリを不要に増やすため採用しない。
