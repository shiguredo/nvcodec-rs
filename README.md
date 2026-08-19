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
    FnEncodeHandler, H264EncoderConfig, Preset, TuningInfo, RateControlMode,
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
let encoder = Encoder::new(config, FnEncodeHandler::new(move |frame: Result<EncodedFrame<()>, Error>| {
    let _ = tx.send(frame);
}))?;

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
encoder.flush()?;

// エンコード済みフレームを取得
for frame in rx.try_iter() {
    let frame = frame?;
    println!("encoded bytes: {}", frame.data().len());
}
```

### デコード

```rust
use std::sync::mpsc;
use shiguredo_nvcodec::{DecodedFrame, Decoder, DecoderCodec, DecoderConfig, Error, FnDecodeHandler, SurfaceFormat};

let config = DecoderConfig {
    codec: DecoderCodec::H264,
    device_id: 0,
    max_num_decode_surfaces: 20,
    max_display_delay: 0,
    surface_format: SurfaceFormat::Nv12,
};

let (tx, rx) = mpsc::sync_channel(4);
let decoder = Decoder::new(config, FnDecodeHandler::new(move |frame: Result<DecodedFrame<()>, Error>| {
    let _ = tx.send(frame);
}))?;

// エンコード済みデータをデコード
decoder.decode(&encoded_data, ())?;

// 全デコード完了を待機
decoder.flush()?;

// デコード済みフレームを取得
for frame in rx.try_iter() {
    let frame = frame?;
    // NV12 フォーマットのデコード結果を取得
    let y_plane = frame.y_plane();
    let uv_plane = frame.uv_plane();
    println!("Y: {}, UV: {}", y_plane.len(), uv_plane.len());
}
```

`DecoderConfig.max_num_decode_surfaces` は、デコードサーフェス数の上限を指定する。`0` は指定できず、`Decoder::new` が設定エラーとして拒否する。NVDEC が正しいデコードに必要な最小サーフェス数 (`CUVIDEOFORMAT.min_num_decode_surfaces`) がこの上限を超える場合は、`decode()` 中の sequence callback で既存 decoder を破棄する前にエラーが返る。実際に割り当てられるサーフェス数は常にこの上限以下になる。

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
let mut encoder = Encoder::new(config, FnEncodeHandler::new(move |_frame| {
    // エンコード結果の処理
}))?;

// 動的に解像度を変更
encoder.reconfigure(ReconfigureParams {
    width: Some(1280),
    height: Some(720),
    ..Default::default()
})?;
```

### デコーダー

ストリーム中に解像度が変わる場合、`DecoderConfig.reconfigure_enabled` で処理方式を選べます。最大解像度を利用者が指定する必要はありません。

- `reconfigure_enabled: false` (推奨値) は従来方式で、シーケンス変更ごとに decoder を破棄して再作成します。
- `reconfigure_enabled: true` は、現在の decoder session の上限 (作成時または再作成時の coded サイズ) 以内の解像度変化を `cuvidReconfigureDecoder` による in-place 再構成で処理し、上限を超える拡大やコーデック情報の変化は再作成で処理します。
- `reconfigure_enabled: true` は `max_display_delay > 0` と組み合わせられません (組み合わせた場合は `Decoder::new` が設定エラーを返します)。

`DecodedFrame` はフレームごとに `width()` / `height()` を持っているので、フレームごとにサイズを確認してください。

フレームの寸法と画素データは、そのフレームの表示領域 (display area) に一致します。`width()` / `height()` は表示領域の寸法を返します。画素データは行矩形ではなく各行が stride を持つバッファで、Y は `y_plane()[y * y_stride() + x]`、UV はインターリーブされたクロマを `uv_plane()` からアクセスしてください。`uv_stride()` は `width()` より大きいことがあります。詳細な出力契約は `DecodedFrame` の rustdoc を参照してください。

```rust
// 解像度が変わっても同じデコーダーで継続可能
let (tx, rx) = mpsc::sync_channel(4);
let decoder = Decoder::new(config, FnDecodeHandler::new(move |frame: Result<DecodedFrame<u32>, Error>| {
    let _ = tx.send(frame);
}))?;

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
| 仕組み | `reconfigure()` で明示的に変更 | `reconfigure_enabled` に応じて再構成 / 再作成を使い分け |
| 利用者の操作 | `ReconfigureParams` で新解像度を指定 | `reconfigure_enabled` を指定するのみ (既定は従来の再作成) |
| 制約 | `max_encode_width` / `max_encode_height` 以内 | session 上限 (作成時または再作成時の coded サイズ) 以内は再構成、超過は再作成 |
| 超えた場合 | エンコーダーを作り直す | 自動対応 |

## 統計値の取得

`Decoder::stats()` / `Encoder::stats()` で、デコーダー / エンコーダーの内部状態を統計値として取得できます。

統計値は `Counter` 型 (通算値) と `Gauge` 型 (時点値) で表現され、`get()` で現在値を読み出します。`stats()` が返すのは共有統計値への参照であり、読み出すたびに最新値が読めます (値を保持したい場合は `clone()` でスナップショットを取得)。

- counter: 単調増加する通算値 (デコーダー作成回数、"encoder buffer is full" エラー発生回数等)
- gauge: 現在値を表す時点値 (in-flight 上限等)

```rust
// エンコーダーの統計値を取得
let stats = encoder.stats();
println!(
    "encoder buffer full count: {}",
    stats.total_encoder_buffer_full_count.get()
);

// in-flight 上限に基づく flush 制御のレシピ
// max_in_flight_frames を超えて encode を連続呼び出しすると
// "encoder buffer is full" エラーになるため、
// 送信フレーム数が上限に達するたびに flush する
let max_in_flight_frames = encoder.stats().max_in_flight_frames.get();
let mut in_flight = 0;
for nv12_data in frame_stream {
    encoder.encode(&nv12_data, &options, ())?;
    in_flight += 1;
    if in_flight >= max_in_flight_frames {
        encoder.flush()?;
        in_flight = 0;
    }
}
encoder.flush()?;
```

デコーダーも同様に `decoder.stats()` で統計値を取得できます。

```rust
let stats = decoder.stats();
println!(
    "decode calls: {}, output frames: {}, in flight: {}",
    stats.total_decode_count.get(),
    stats.total_output_frame_count.get(),
    stats.in_flight_frames()
);
```

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
