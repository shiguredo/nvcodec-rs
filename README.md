# nvcodec-rs

[![crates.io](https://img.shields.io/crates/v/shiguredo_nvcodec.svg)](https://crates.io/crates/shiguredo_nvcodec)
[![docs.rs](https://docs.rs/shiguredo_nvcodec/badge.svg)](https://docs.rs/shiguredo_nvcodec)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![GitHub Actions](https://github.com/shiguredo/nvcodec-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/shiguredo/nvcodec-rs/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/shiguredo)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## 概要

[NVIDIA Video Codec SDK](https://developer.nvidia.com/video-codec-sdk) を利用したハードウェアビデオエンコーダーおよびデコーダーの Rust バインディングです。

CUDA ドライバー API を実行時に動的ロード (`dlopen`) するため、ビルド時に CUDA Toolkit のリンクは不要です。

## 特徴

- NVENC によるハードウェアエンコード (H.264 / H.265 / AV1)
- NVCUVID によるハードウェアデコード (H.264 / H.265 / AV1 / VP8 / VP9 / JPEG)
- CUDA ライブラリの実行時動的ロード (ビルド時の CUDA Toolkit リンク不要)
- エンコーダー / デコーダーのケーパビリティクエリ
- エンコード入力バッファフォーマット選択 (NV12 / YV12 / I420 / YUV444 / 10bit / ARGB / ABGR)
- デコード出力サーフェスフォーマット選択 (NV12 / P016 / YUV444 / NV16 / P216)
- フレーム単位のエンコードオプション (IDR フレーム強制、SPS/PPS 出力)
- エンコーダーのランタイム再構成 (解像度、ビットレート、フレームレート変更)
- デコーダーの動的解像度変更の自動対応
- CUDA デバイス列挙
- CUDA ストリーム管理
- 2D メモリコピー、ピッチ付きメモリ割り当て

## 動作要件

- Linux (x86_64)
- NVIDIA GPU (Kepler 世代以降)
- NVIDIA ドライバー (CUDA ドライバー API を含む)
- NVIDIA Video Codec SDK 13.0 以降のヘッダーファイル (ビルド時)

## ビルド

CUDA Toolkit がインストールされている Linux 環境でビルドしてください。

```bash
cargo build
```

### docs.rs 向けビルド

CUDA Toolkit がない環境では、同梱のスタブヘッダーを使って docs.rs 向けのドキュメント生成のみ可能です。

```bash
DOCS_RS=1 cargo doc --no-deps
```

## 使い方

### エンコード

```rust
use std::sync::mpsc;
use shiguredo_nvcodec::{
    BufferFormat, CodecConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig, Error,
    H264EncoderConfig, Preset, TuningInfo, RateControlMode,
};

let config = EncoderConfig {
    codec: CodecConfig::H264(H264EncoderConfig {
        profile: None,
        idr_period: None,
    }),
    width: 1920,
    height: 1080,
    max_encode_width: None,
    max_encode_height: None,
    framerate_num: 30,
    framerate_den: 1,
    average_bitrate: Some(5_000_000),
    preset: Preset::P4,
    tuning_info: TuningInfo::LOW_LATENCY,
    rate_control_mode: RateControlMode::Cbr,
    gop_length: None,
    frame_interval_p: 1,
    buffer_format: BufferFormat::Nv12,
    device_id: 0,
};

let (tx, rx) = mpsc::sync_channel(4);
let encoder = Encoder::new(config, move |frame: Result<EncodedFrame<()>, Error>| {
    let _ = tx.send(frame);
})?;

// NV12 フレームデータをエンコード
let options = EncodeOptions {
    force_intra: false,
    force_idr: false,
    output_spspps: false,
};
encoder.encode(&nv12_data, &options, ())?;

// IDR フレームを強制してエンコード
let force_idr_options = EncodeOptions {
    force_intra: false,
    force_idr: true,
    output_spspps: false,
};
encoder.encode(&nv12_data, &force_idr_options, ())?;

// 全エンコード完了を待機
encoder.flush();

// エンコード済みフレームを取得
for frame in rx.try_iter() {
    let frame = frame?;
    println!("encoded bytes: {}", frame.data().len());
}
```

### デコード

```rust
use std::sync::mpsc;
use shiguredo_nvcodec::{DecodedFrame, Decoder, DecoderCodec, DecoderConfig, Error, SurfaceFormat};

let config = DecoderConfig {
    codec: DecoderCodec::H264,
    device_id: 0,
    max_num_decode_surfaces: 20,
    max_display_delay: 0,
    surface_format: SurfaceFormat::Nv12,
};

let (tx, rx) = mpsc::sync_channel(4);
let decoder = Decoder::new(config, move |frame: Result<DecodedFrame<()>, Error>| {
    let _ = tx.send(frame);
})?;

// エンコード済みデータをデコード
decoder.decode(&encoded_data, ())?;

// 全デコード完了を待機
decoder.flush();

// デコード済みフレームを取得
for frame in rx.try_iter() {
    let frame = frame?;
    // NV12 フォーマットのデコード結果を取得
    let y_plane = frame.y_plane();
    let uv_plane = frame.uv_plane();
    println!("Y: {}, UV: {}", y_plane.len(), uv_plane.len());
}
```

### エンコーダーケーパビリティクエリ

```rust
use shiguredo_nvcodec::{EncoderCodec, query_encoder_caps};

let caps = query_encoder_caps(EncoderCodec::H264, 0)?;
println!("max width: {}", caps.width_max);
println!("max height: {}", caps.height_max);
println!("10-bit encode: {}", caps.support_10bit_encode);
```

### デコーダーケーパビリティクエリ

```rust
use shiguredo_nvcodec::{DecoderCodec, query_decoder_caps};

let caps = query_decoder_caps(DecoderCodec::H264, 0)?;
println!("supported: {}", caps.is_supported);
println!("max: {}x{}", caps.max_width, caps.max_height);
```

### CUDA デバイス列挙

```rust
use shiguredo_nvcodec;

let count = shiguredo_nvcodec::device_count()?;
for i in 0..count {
    let name = shiguredo_nvcodec::device_name(i)?;
    println!("GPU {}: {}", i, name);
}
```

## サポートコーデック

### エンコード

| コーデック | `CodecConfig` |
|-----------|--------------|
| H.264     | `CodecConfig::H264(H264EncoderConfig)` |
| H.265     | `CodecConfig::Hevc(HevcEncoderConfig)` |
| AV1       | `CodecConfig::Av1(Av1EncoderConfig)` |

### デコード

| コーデック | `DecoderCodec` |
|-----------|--------------|
| H.264     | `DecoderCodec::H264` |
| H.265     | `DecoderCodec::Hevc` |
| AV1       | `DecoderCodec::Av1` |
| VP8       | `DecoderCodec::Vp8` |
| VP9       | `DecoderCodec::Vp9` |
| JPEG      | `DecoderCodec::Jpeg` |

## サポートフォーマット

### エンコード入力バッファフォーマット (`BufferFormat`)

| フォーマット | `BufferFormat` | 説明 |
|---|---|---|
| NV12 | `BufferFormat::Nv12` | Semi-Planar YUV 4:2:0 8bit |
| YV12 | `BufferFormat::Yv12` | Planar YUV 4:2:0 8bit (Y+V+U) |
| IYUV (I420) | `BufferFormat::Iyuv` | Planar YUV 4:2:0 8bit (Y+U+V) |
| YUV444 | `BufferFormat::Yuv444` | Planar YUV 4:4:4 8bit |
| YUV420 10bit | `BufferFormat::Yuv420_10bit` | Semi-Planar YUV 4:2:0 10bit |
| YUV444 10bit | `BufferFormat::Yuv444_10bit` | Planar YUV 4:4:4 10bit |
| ARGB | `BufferFormat::Argb` | Packed A8R8G8B8 |
| ABGR | `BufferFormat::Abgr` | Packed A8B8G8R8 |
| ARGB 10bit | `BufferFormat::Argb10` | Packed A2R10G10B10 |
| ABGR 10bit | `BufferFormat::Abgr10` | Packed A2B10G10R10 |

### デコード出力サーフェスフォーマット (`SurfaceFormat`)

| フォーマット | `SurfaceFormat` | 説明 |
|---|---|---|
| NV12 | `SurfaceFormat::Nv12` | Semi-Planar YUV 4:2:0 8bit |
| P016 | `SurfaceFormat::P016` | Semi-Planar YUV 4:2:0 16bit |
| YUV444 | `SurfaceFormat::Yuv444` | Planar YUV 4:4:4 8bit |
| YUV444 16bit | `SurfaceFormat::Yuv444_16bit` | Planar YUV 4:4:4 16bit |
| NV16 | `SurfaceFormat::Nv16` | Semi-Planar YUV 4:2:2 8bit |
| P216 | `SurfaceFormat::P216` | Semi-Planar YUV 4:2:2 16bit |

## 動的解像度変更

WebRTC やアダプティブビットレートストリーミングなど、ストリーム中に解像度が変わるユースケースに対応しています。

### エンコーダー

`reconfigure()` で解像度を変更できます。エンコーダーの作り直しは不要です。

ただし、初期化時に `max_encode_width` / `max_encode_height` を設定しておく必要があります。新しい解像度がこの範囲内であれば変更可能です。

```rust
use shiguredo_nvcodec::ReconfigureParams;

// 作成時に最大解像度を指定
let config = EncoderConfig {
    width: 1920,
    height: 1080,
    max_encode_width: Some(3840),   // 4K まで変更可能
    max_encode_height: Some(2160),
    // ...
};
let mut encoder = Encoder::new(config)?;

// 動的に解像度を変更
encoder.reconfigure(ReconfigureParams {
    width: Some(1280),
    height: Some(720),
    ..Default::default()
})?;
```

### デコーダー

利用者側の操作は不要です。ストリーム中に解像度が変わった場合、内部で自動的にデコーダーが再作成されます。

`DecodedFrame` はフレームごとに `width()` / `height()` を持っているので、フレームごとにサイズを確認してください。

```rust
// 解像度が変わっても同じデコーダーで継続可能
let (tx, rx) = mpsc::sync_channel(4);
let decoder = Decoder::new(config, move |frame: Result<DecodedFrame<u32>, Error>| {
    let _ = tx.send(frame);
})?;

decoder.decode(&data_1080p, 0)?;
let frame = rx.recv()??;
assert_eq!(frame.width(), 1920);

decoder.decode(&data_720p, 1)?;
let frame = rx.recv()??;
assert_eq!(frame.width(), 1280);  // 自動的に変更される
```

### まとめ

| | エンコーダー | デコーダー |
|---|---|---|
| 仕組み | `reconfigure()` で明示的に変更 | パーサーが自動検出して再作成 |
| 利用者の操作 | `ReconfigureParams` で新解像度を指定 | 不要 |
| 制約 | `max_encode_width` / `max_encode_height` 以内 | なし |
| 超えた場合 | エンコーダーを作り直す | 自動対応 |

## ライセンス

Apache License 2.0

```text
Copyright 2026-2026, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

## NVIDIA Video Codec SDK

<https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/index.html>

<https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/license/index.html>

```text
“This software contains source code provided by NVIDIA Corporation.”
```
