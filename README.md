# nvcodec-rs

[![crates.io](https://img.shields.io/crates/v/shiguredo_nvcodec.svg)](https://crates.io/crates/shiguredo_nvcodec)
[![docs.rs](https://docs.rs/shiguredo_nvcodec/badge.svg)](https://docs.rs/shiguredo_nvcodec)
[![CI](https://github.com/shiguredo/nvcodec-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/shiguredo/nvcodec-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## shiguredo_nvcodec について

[NVIDIA Video Codec SDK](https://developer.nvidia.com/video-codec-sdk) を利用したハードウェアビデオエンコーダーおよびデコーダーの Rust バインディングです。

CUDA ドライバー API を実行時に動的ロード (`dlopen`) するため、ビルド時に CUDA Toolkit のリンクは不要です。

## 特徴

- NVENC によるハードウェアエンコード (H.264 / H.265 / AV1)
- NVCUVID によるハードウェアデコード (H.264 / H.265 / AV1 / VP8 / VP9 / JPEG)
- CUDA ライブラリの実行時動的ロード (ビルド時の CUDA Toolkit リンク不要)
- エンコーダー / デコーダーの能力クエリ
- エンコーダーのランタイム再構成 (ビットレート、フレームレート変更)
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
use shiguredo_nvcodec::{
    CodecConfig, Encoder, EncoderConfig, H264EncoderConfig,
    Preset, TuningInfo, RateControlMode,
};

let config = EncoderConfig {
    codec: CodecConfig::H264(H264EncoderConfig::default()),
    width: 1920,
    height: 1080,
    fps_numerator: 30,
    fps_denominator: 1,
    target_bitrate: Some(5_000_000),
    preset: Preset::P4,
    tuning_info: TuningInfo::LOW_LATENCY,
    rate_control_mode: RateControlMode::Cbr,
    ..Default::default()
};

let mut encoder = Encoder::new(config)?;
// NV12 フレームデータをエンコード
encoder.encode(&nv12_data)?;

// エンコード済みフレームを取得
if let Some(encoded) = encoder.next_frame() {
    println!("encoded bytes: {}", encoded.data().len());
}
```

### デコード

```rust
use shiguredo_nvcodec::{Decoder, DecoderCodec, DecoderConfig};

let config = DecoderConfig {
    codec: DecoderCodec::H264,
    ..Default::default()
};
let mut decoder = Decoder::new(config)?;

// エンコード済みデータをデコード
decoder.decode(&encoded_data)?;
decoder.finish()?;

// デコード済みフレームを取得
while let Some(frame) = decoder.next_frame()? {
    // NV12 フォーマットのデコード結果を取得
    let y_plane = frame.y_plane();
    let uv_plane = frame.uv_plane();
    println!("Y: {}, UV: {}", y_plane.len(), uv_plane.len());
}
```

### エンコーダー能力クエリ

```rust
use shiguredo_nvcodec::{Encoder, EncoderCodec};

let caps = Encoder::query_caps(EncoderCodec::H264, 0)?;
println!("max width: {}", caps.width_max);
println!("max height: {}", caps.height_max);
println!("10-bit encode: {}", caps.support_10bit_encode);
```

### デコーダー能力クエリ

```rust
use shiguredo_nvcodec::{Decoder, DecoderCodec};

let caps = Decoder::query_caps(DecoderCodec::H264, 0)?;
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
