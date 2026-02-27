# 0005 デコーダーの出力サーフェスフォーマットを設定可能にする

## 概要

現在 `cudaVideoSurfaceFormat_NV12` がハードコードされている。`DecoderConfig` に `surface_format` フィールドを追加し、NVDEC がサポートする出力サーフェスフォーマットを選択可能にする。

## 背景

現在の実装:

- `OutputFormat` は `cudaVideoSurfaceFormat_NV12` 固定
- デコード結果は常に NV12 形式で返される
- 10bit コンテンツや 4:4:4 コンテンツを正しく出力できない

## 新しい API

```rust
/// デコーダー出力サーフェスフォーマット (NVDEC: cudaVideoSurfaceFormat)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceFormat {
    /// Semi-Planar YUV 4:2:0 8bit [Y plane + interleaved UV plane]
    Nv12,
    /// Semi-Planar YUV 4:2:0 16bit [Y plane + interleaved UV plane]
    P016,
    /// Planar YUV 4:4:4 8bit [Y plane + U plane + V plane]
    Yuv444,
    /// Planar YUV 4:4:4 16bit [Y plane + U plane + V plane]
    Yuv444_16bit,
    /// Semi-Planar YUV 4:2:2 8bit [Y plane + interleaved UV plane]
    Nv16,
    /// Semi-Planar YUV 4:2:2 16bit [Y plane + interleaved UV plane]
    P216,
}
```

```rust
pub struct DecoderConfig {
    pub codec: DecoderCodec,
    pub device_id: i32,
    pub max_num_decode_surfaces: u32,
    pub max_display_delay: u32,
    /// 出力サーフェスフォーマット (NVDEC: OutputFormat)
    pub surface_format: SurfaceFormat,
}
```

## 設計方針

- `SurfaceFormat` enum を新設する（NVDEC SDK の `cudaVideoSurfaceFormat` に対応）
- `DecoderConfig` に `surface_format: SurfaceFormat` フィールドを追加する
- `CUVIDDECODECREATEINFO.OutputFormat` にそのまま渡す
- コーデックとフォーマットの組み合わせに制約がある（ケーパビリティクエリの `nOutputFormatMask` で確認可能）

## NVDEC SDK の cudaVideoSurfaceFormat

| フォーマット | 値 | 説明 |
|---|---|---|
| `cudaVideoSurfaceFormat_NV12` | 0 | Semi-Planar YUV 4:2:0 (8bit) |
| `cudaVideoSurfaceFormat_P016` | 1 | Semi-Planar YUV 4:2:0 (16bit) |
| `cudaVideoSurfaceFormat_YUV444` | 2 | Planar YUV 4:4:4 (8bit) |
| `cudaVideoSurfaceFormat_YUV444_16Bit` | 3 | Planar YUV 4:4:4 (16bit) |
| `cudaVideoSurfaceFormat_NV16` | 4 | Semi-Planar YUV 4:2:2 (8bit) |
| `cudaVideoSurfaceFormat_P216` | 5 | Semi-Planar YUV 4:2:2 (16bit) |

## 使用例

### NV12 でデコード (8bit H.264)

```rust
let config = DecoderConfig {
    codec: DecoderCodec::H264,
    device_id: 0,
    max_num_decode_surfaces: 20,
    max_display_delay: 0,
    surface_format: SurfaceFormat::Nv12,
};
let mut decoder = Decoder::new(config)?;
```

### P016 でデコード (10bit HEVC)

```rust
let config = DecoderConfig {
    codec: DecoderCodec::Hevc,
    device_id: 0,
    max_num_decode_surfaces: 20,
    max_display_delay: 0,
    surface_format: SurfaceFormat::P016,
};
let mut decoder = Decoder::new(config)?;
```

### YUV444 でデコード (4:4:4 H.264)

```rust
let config = DecoderConfig {
    codec: DecoderCodec::H264,
    device_id: 0,
    max_num_decode_surfaces: 20,
    max_display_delay: 0,
    surface_format: SurfaceFormat::Yuv444,
};
let mut decoder = Decoder::new(config)?;
```

## 変更対象ファイル

- `src/decode.rs`: `SurfaceFormat` enum 追加、`DecoderConfig` にフィールド追加、`OutputFormat` のハードコード削除
- `src/lib.rs`: `SurfaceFormat` を pub use に追加
- `build.rs`: スタブ定義にサーフェスフォーマット定数を追加
- `README.md`: デコードセクションのコード例を更新
- `CHANGES.md`: 変更履歴を追加
