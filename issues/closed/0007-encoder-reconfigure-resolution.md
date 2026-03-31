# 0007 エンコーダーの動的解像度変更に対応する

## 概要

`ReconfigureParams` に `width` / `height` を追加し、エンコーダーを作り直さずに解像度を動的に変更できるようにする。

## 背景

現在の `ReconfigureParams` はビットレートとフレームレートの変更のみ対応している。NVENC SDK の `NvEncReconfigureEncoder` は解像度変更もサポートしているが、現在の実装では利用できない。

WebRTC のように送信側が解像度を動的に変更するユースケースで必要になる。

## 現在の実装

```rust
pub struct ReconfigureParams {
    pub framerate_num: Option<u32>,
    pub framerate_den: Option<u32>,
    pub average_bitrate: Option<u32>,
    pub max_bitrate: Option<u32>,
}
```

`EncoderConfig` に `max_encode_width` / `max_encode_height` は既にあるが、`ReconfigureParams` で解像度を変更する手段がない。

## NVENC SDK の制約

- `NvEncReconfigureEncoder` で `encodeWidth` / `encodeHeight` を変更できる
- ただし `maxEncodeWidth` / `maxEncodeHeight`（初期化時に指定）を超えてはならない
- 超えた場合は `NV_ENC_ERR_INVALID_PARAM` が返る

## 設計方針

- `ReconfigureParams` に `width: Option<u32>` / `height: Option<u32>` を追加する
- 解像度変更時に `Encoder` の `width` / `height` / `expected_frame_size` を更新する
- `max_encode_width` / `max_encode_height` を超える場合は SDK 呼び出し前にエラーを返す

## 変更対象ファイル

- `src/encode.rs`: `ReconfigureParams` にフィールド追加、`reconfigure_inner` の修正
- `CHANGES.md`: 変更履歴を追加

## 完了内容

- `ReconfigureParams` に `width: Option<u32>` / `height: Option<u32>` を追加
- `reconfigure_inner` で `maxEncodeWidth` / `maxEncodeHeight` を超える場合に SDK 呼び出し前にエラーを返すバリデーションを追加
- 解像度変更時に `Encoder` の `width` / `height` / `expected_frame_size` を更新
- `Encoder` 構造体に `buffer_format_enum: BufferFormat` フィールドを追加（`frame_size` 再計算用）
