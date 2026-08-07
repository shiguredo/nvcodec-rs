# 0028-change-decoder-num-decode-surfaces-per-codec

- Created: 2026-08-07
- Branch: feature/change-decoder-num-decode-surfaces-per-codec

## 目的

`cuvidCreateDecoder` / `cuvidReconfigureDecoder` に渡す `ulNumDecodeSurfaces` を、NVDEC parser 報告の `format.min_num_decode_surfaces` ではなくコーデック仕様に基づいた推奨値に引き上げる。

参照フレーム数の多い HEVC / VP9 / AV1 で `min_num_decode_surfaces` (通常 8-9) では DPB が不足する場面があり、`cuvidReconfigureDecoder` 直後の `cuvidDecodePicture` が `CUDA_ERROR_INVALID_VALUE` を返すリスクがある。NVIDIA 公式サンプル `NvDecoder::GetNumDecodeSurfaces` に準拠した推奨値を採用してリスクを回避する。

## 現状

`src/decode.rs` の `create_decoder` および `handle_video_sequence_inner` (0024 マージ後は reconfigure 分岐も含む) で、`ulNumDecodeSurfaces = format.min_num_decode_surfaces as u64` としている。`min_num_decode_surfaces` は NVDEC parser がストリーム解析結果から計算する最小値で、概ね以下の値になる:

- H.264: プロファイル依存 (概ね 4-16)
- HEVC: 8
- VP9: 9
- AV1: 9
- VP8: 3-4
- JPEG: 1

NVIDIA 公式サンプル `NvDecoder::GetNumDecodeSurfaces` の値:

- H.264: 20
- HEVC: 20
- VP9: 12
- AV1: 12 (サンプル default = 8 だが、AV1 仕様上の 8 参照 + 現在フレームで min 9、余裕を持たせて VP9 相当の 12 が妥当)
- VP8: 8
- JPEG: 1

サンプル値が `min_num_decode_surfaces` を上回るのは、参照フレーム数の多い codec で min ギリギリだと DPB 不足で decode 失敗するリスクがあるため。

0024 の PR 検証時、HEVC / VP9 / AV1 で縮小方向 reconfigure 直後の `cuvidDecodePicture` が `CUDA_ERROR_INVALID_VALUE` を返す現象を観測した (根本原因はテストデータの解像度が hardware min を下回っていたことだったが、DPB 不足でも同種の失敗が起きうる点は独立に対応する価値がある)。

## 設計方針

### helper 関数の追加

```rust
/// コーデック別の推奨デコードサーフェス数を返す
///
/// NVIDIA 公式サンプル NvDecoder::GetNumDecodeSurfaces に準拠
fn get_codec_num_decode_surfaces(codec: sys::cudaVideoCodec) -> u32 {
    match codec {
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_VP9 => 12,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC => 20,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_H264 => 20,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_AV1 => 12,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_VP8 => 8,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_JPEG => 1,
        _ => 8, // NVIDIA サンプル default
    }
}

/// format と codec からデコードサーフェス数を決定する
///
/// parser 報告の最小値と codec 別推奨値の大きい方を採用し、
/// パーサ作成時に指定した `ulMaxNumDecodeSurfaces` を上限として clamp する
fn effective_num_decode_surfaces(format: &sys::CUVIDEOFORMAT, max_allowed: u32) -> u32 {
    let min = format.min_num_decode_surfaces as u32;
    let codec_recommended = get_codec_num_decode_surfaces(format.codec);
    min.max(codec_recommended).min(max_allowed.max(min))
}
```

### 適用箇所

以下 3 箇所で `format.min_num_decode_surfaces` から `effective_num_decode_surfaces(...)` に置き換える (3 番目は 0024 マージ後に追加される reconfigure 分岐):

1. `create_decoder` の `create_info.ulNumDecodeSurfaces`
2. `handle_video_sequence_inner` の戻り値 (parser コールバック返却値)
3. `handle_video_sequence_inner` の reconfigure 分岐の `reconfigure_info.ulNumDecodeSurfaces`

3 箇所で同じ値を使う必要がある (parser がこの値で `curr_pic_idx` を割り当て、decoder がその範囲で DPB を確保するため)。

### `DecoderState` への `max_num_decode_surfaces` 保持

`effective_num_decode_surfaces` の上限として `config.max_num_decode_surfaces` (parser 作成時に指定した `ulMaxNumDecodeSurfaces`) を渡す必要がある。現状 `DecoderState` は保持していないため、`u32` フィールドを追加する。

### `reconfigure` の制約

`cuvidReconfigureDecoder` は `ulNumDecodeSurfaces` を後から増やすことはできない (NVDEC SDK の制約)。そのため、初回 `cuvidCreateDecoder` の段階から推奨値を渡しておく必要がある。

## 利用側への影響

**破壊的変更ではない** (公開 API シグネチャ変更なし) が、**内部挙動が変わる**:

- HEVC / VP9 / AV1 で DPB プールが大きくなる (最大 `max_num_decode_surfaces` まで)
- GPU メモリ使用量が増える (surface サイズ × 増分)
  - SD/HD (320x240, 640x480 等): 影響軽微 (数 MB 増)
  - 4K (3840x2160 NV12): 増分大 (surface 1 枚 ≈ 12MB、HEVC で 8→20 = 12 surface 増 = 約 150MB 増)

利用側が GPU メモリを絞りたい場合の escape hatch:

- `DecoderConfig::max_num_decode_surfaces` を推奨値未満に設定すれば、`effective_num_decode_surfaces` の clamp によって元の `min_num_decode_surfaces` 相当に落ちる
  - 例: HEVC で `max_num_decode_surfaces = 8` を設定すれば、`effective = max(8, 20).min(max(8, 8)) = 20.min(8) = 8` となり従来相当

この挙動と escape hatch は rustdoc / README / SKILL.md に明記する。

## 完了条件

- `get_codec_num_decode_surfaces` / `effective_num_decode_surfaces` helper が追加され、`cuvidCreateDecoder` / `cuvidReconfigureDecoder` / sequence callback 戻り値の 3 箇所で使われている
- `DecoderState` に `max_num_decode_surfaces: u32` フィールドが追加され、`config.max_num_decode_surfaces` を保持している
- helper の単体テストがある (推奨値・clamp・min 優先の境界を GPU 不要で検証)
- `DecoderConfig::max_num_decode_surfaces` の rustdoc に codec 別推奨値との相互作用と escape hatch が追記されている
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` に GPU メモリ使用量が増える旨と escape hatch のレシピが追記されている
- `CHANGES.md` に `[CHANGE]` エントリが追加されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — `get_codec_num_decode_surfaces` / `effective_num_decode_surfaces` helper 追加、`DecoderState` に `max_num_decode_surfaces` フィールド追加、`create_decoder` / `handle_video_sequence_inner` の 3 箇所で置き換え、helper の単体テスト追加
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` — GPU メモリ増加と escape hatch のレシピ追記
- `CHANGES.md` — 追記例:
  - `- [CHANGE] デコーダーの ulNumDecodeSurfaces を codec 別推奨値 (H.264/HEVC=20, VP9/AV1=12, VP8=8, JPEG=1) に引き上げる`
  - `  - GPU メモリ使用量が増えるが、max_num_decode_surfaces を推奨値未満に設定することで従来相当に落とせる`
  - `  - @担当者`

## 関連 issue

- 0024: 本 issue の実装検証で判明した派生課題。0024 のブランチで先行実装 (commit `b54c66d`) されていたが、独立して意味論を議論するために本 issue に切り出す (0024 側では revert する)。本 issue の 3 番目の適用箇所 (reconfigure 分岐) は 0024 マージ後に追加される
