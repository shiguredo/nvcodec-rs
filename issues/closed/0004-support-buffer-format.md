# 0004 エンコーダーのバッファフォーマットをハードコードから設定可能にする

## 概要

現在 `NV_ENC_BUFFER_FORMAT_NV12` がハードコードされている。`EncoderConfig` に `buffer_format` フィールドを追加し、NVENC SDK がサポートする入力バッファフォーマットを選択可能にする。

## 背景

現在の実装:

- `buffer_format` は `NV_ENC_BUFFER_FORMAT_NV12` 固定
- `encode()` の引数名が `nv12_data` になっている
- サイズ検証が NV12 前提 (`width * height * 3 / 2`)

## 新しい API

```rust
/// 入力バッファフォーマット (NVENC: NV_ENC_BUFFER_FORMAT)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferFormat {
    /// Semi-Planar YUV 4:2:0 [Y plane + interleaved UV plane]
    Nv12,
    /// Planar YUV 4:2:0 [Y plane + V plane + U plane]
    Yv12,
    /// Planar YUV 4:2:0 [Y plane + U plane + V plane] (I420)
    Iyuv,
    /// Planar YUV 4:4:4 [Y plane + U plane + V plane]
    Yuv444,
    /// 10bit Semi-Planar YUV 4:2:0 [Y plane + interleaved UV plane]
    Yuv420_10bit,
    /// 10bit Planar YUV 4:4:4 [Y plane + U plane + V plane]
    Yuv444_10bit,
    /// 8bit Packed A8R8G8B8
    Argb,
    /// 8bit Packed A8B8G8R8
    Abgr,
    /// 10bit Packed A2R10G10B10
    Argb10,
    /// 10bit Packed A2B10G10R10
    Abgr10,
}
```

```rust
pub struct EncoderConfig {
    // 既存フィールドは省略
    /// 入力バッファフォーマット (NVENC: bufferFormat)
    pub buffer_format: BufferFormat,
}
```

## 設計方針

- `BufferFormat` enum を新設する（NVENC SDK の `NV_ENC_BUFFER_FORMAT` に対応）
- `EncoderConfig` に `buffer_format: BufferFormat` フィールドを追加する
- `encode()` の引数名を `nv12_data` から `frame_data` に変更する
- サイズ検証を `BufferFormat` ごとに計算する
- NVENC SDK がサポートしない形式（YUY2 など）は含めない
- `NV_ENC_BUFFER_FORMAT_U8` / `NV_ENC_BUFFER_FORMAT_NV16` / `NV_ENC_BUFFER_FORMAT_P210` / `NV_ENC_BUFFER_FORMAT_AYUV` は利用頻度が低いため初回スコープ外とする

## NVENC SDK の NV_ENC_BUFFER_FORMAT

| フォーマット | 値 | 説明 |
|---|---|---|
| `NV_ENC_BUFFER_FORMAT_NV12` | 0x00000001 | Semi-Planar YUV 4:2:0 |
| `NV_ENC_BUFFER_FORMAT_YV12` | 0x00000010 | Planar YUV 4:2:0 (Y+V+U) |
| `NV_ENC_BUFFER_FORMAT_IYUV` | 0x00000100 | Planar YUV 4:2:0 (Y+U+V) = I420 |
| `NV_ENC_BUFFER_FORMAT_YUV444` | 0x00001000 | Planar YUV 4:4:4 |
| `NV_ENC_BUFFER_FORMAT_YUV420_10BIT` | 0x00010000 | 10bit Semi-Planar YUV 4:2:0 |
| `NV_ENC_BUFFER_FORMAT_YUV444_10BIT` | 0x00100000 | 10bit Planar YUV 4:4:4 |
| `NV_ENC_BUFFER_FORMAT_ARGB` | 0x01000000 | 8bit Packed A8R8G8B8 |
| `NV_ENC_BUFFER_FORMAT_ABGR` | 0x10000000 | 8bit Packed A8B8G8R8 |
| `NV_ENC_BUFFER_FORMAT_ARGB10` | 0x02000000 | 10bit Packed A2R10G10B10 |
| `NV_ENC_BUFFER_FORMAT_ABGR10` | 0x20000000 | 10bit Packed A2B10G10R10 |

## フォーマットごとのフレームサイズ計算

| フォーマット | サイズ |
|---|---|
| NV12 / YV12 / IYUV | `width * height * 3 / 2` |
| YUV444 | `width * height * 3` |
| YUV420_10BIT | `width * height * 3` (2 bytes/pixel) |
| YUV444_10BIT | `width * height * 6` (2 bytes/pixel) |
| ARGB / ABGR | `width * height * 4` |
| ARGB10 / ABGR10 | `width * height * 4` |

## 使用例

### NV12 でエンコード

```rust
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
let mut encoder = Encoder::new(config)?;

let options = EncodeOptions {
    force_intra: false,
    force_idr: false,
    output_spspps: false,
};
// NV12: width * height * 3 / 2 バイト
encoder.encode(&nv12_data, &options)?;
```

### I420 でエンコード

```rust
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
    buffer_format: BufferFormat::Iyuv,
    device_id: 0,
};
let mut encoder = Encoder::new(config)?;

let options = EncodeOptions {
    force_intra: false,
    force_idr: false,
    output_spspps: false,
};
// I420: width * height * 3 / 2 バイト
encoder.encode(&i420_data, &options)?;
```

### 10bit HEVC でエンコード

```rust
let config = EncoderConfig {
    codec: CodecConfig::Hevc(HevcEncoderConfig {
        profile: Some(HevcProfile::Main10),
        idr_period: None,
    }),
    width: 3840,
    height: 2160,
    max_encode_width: None,
    max_encode_height: None,
    framerate_num: 60,
    framerate_den: 1,
    average_bitrate: Some(20_000_000),
    preset: Preset::P4,
    tuning_info: TuningInfo::LOW_LATENCY,
    rate_control_mode: RateControlMode::Vbr,
    gop_length: None,
    frame_interval_p: 1,
    buffer_format: BufferFormat::Yuv420_10bit,
    device_id: 0,
};
let mut encoder = Encoder::new(config)?;

let options = EncodeOptions {
    force_intra: false,
    force_idr: false,
    output_spspps: false,
};
// YUV420_10BIT: width * height * 3 バイト (2 bytes/pixel)
encoder.encode(&yuv420_10bit_data, &options)?;
```

## 変更対象ファイル

- `src/encode.rs`: `BufferFormat` enum 追加、`EncoderConfig` にフィールド追加、`encode()` の引数名変更、サイズ検証修正
- `src/lib.rs`: `BufferFormat` を pub use に追加
- `build.rs`: スタブ定義にバッファフォーマット定数を追加
- `README.md`: コード例を更新
- `CHANGES.md`: 変更履歴を追加
