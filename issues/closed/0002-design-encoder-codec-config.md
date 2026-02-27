# 0002 エンコーダーのコーデック設定 API 設計

## 概要

`Encoder::new_h264(EncoderConfig)` / `Encoder::new_h265(EncoderConfig)` / `Encoder::new_av1(EncoderConfig)` は
コーデック名がメソッド名に埋め込まれており、`EncoderConfig` の `profile` と `idr_period` は全コーデック共通の
抽象化になっていた。これは NVENC SDK の設計（コーデック別の `NV_ENC_CONFIG_H264` /
`NV_ENC_CONFIG_HEVC` / `NV_ENC_CONFIG_AV1`）と乖離している。

デコーダー側も同様に `Decoder::new_h264` / `Decoder::new_h265` 等のコーデック別コンストラクタが存在していた。

## 方針

`EncoderConfig` の `codec` フィールドに `CodecConfig` enum を持たせる。
enum のバリアントがコーデック名とコーデック固有設定を同時に表現する。
コンストラクタは `Encoder::new(config)` に統一し、旧 API は全て削除する。

デコーダー側は `DecoderCodec` 識別子 enum を導入し、`Decoder::new(config)` に統一する。

## エンコーダー API

### コーデック固有プロファイル enum

```rust
pub enum H264Profile {
    AutoSelect, Baseline, Main, High, High10,
    High422, High444, Stereo, ProgressiveHigh, ConstrainedHigh,
}

pub enum HevcProfile {
    AutoSelect, Main, Main10, Frext,
}

pub enum Av1Profile {
    AutoSelect, Main,
}
```

### コーデック固有設定構造体

```rust
pub struct H264EncoderConfig {
    pub profile: Option<H264Profile>,   // None → Main
    pub idr_period: Option<u32>,        // None → gop_length と同じ
}

pub struct HevcEncoderConfig {
    pub profile: Option<HevcProfile>,
    pub idr_period: Option<u32>,
}

pub struct Av1EncoderConfig {
    pub profile: Option<Av1Profile>,
    pub idr_period: Option<u32>,
}
```

### CodecConfig enum

```rust
pub enum CodecConfig {
    H264(H264EncoderConfig),
    Hevc(HevcEncoderConfig),
    Av1(Av1EncoderConfig),
}
```

### EncoderCodec enum（query_caps 用）

```rust
pub enum EncoderCodec {
    H264,
    Hevc,
    Av1,
}
```

### EncoderConfig

```rust
pub struct EncoderConfig {
    pub codec: CodecConfig,
    pub width: u32,
    pub height: u32,
    pub max_encode_width: Option<u32>,
    pub max_encode_height: Option<u32>,
    pub framerate_num: u32,
    pub framerate_den: u32,
    pub average_bitrate: Option<u32>,
    pub preset: Preset,
    pub tuning_info: TuningInfo,
    pub rate_control_mode: RateControlMode,
    pub gop_length: Option<u32>,
    pub frame_interval_p: u32,
    pub device_id: i32,
}
```

`Default`: `codec = CodecConfig::H264(Default)`, `640x480`, `30fps`, `5Mbps VBR`, `P4`, `LOW_LATENCY`

### Encoder

```rust
impl Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, Error>;
    pub fn query_caps(codec: EncoderCodec, device_id: i32) -> Result<EncoderCaps, Error>;
    pub fn encode(&mut self, nv12_data: &[u8]) -> Result<(), Error>;
    pub fn finish(&mut self) -> Result<(), Error>;
    pub fn next_frame(&mut self) -> Option<EncodedFrame>;
    pub fn get_sequence_params(&mut self) -> Result<Vec<u8>, Error>;
    pub fn reconfigure(&mut self, params: ReconfigureParams) -> Result<(), Error>;
}
```

### 使用例

```rust
let config = EncoderConfig {
    codec: CodecConfig::Hevc(HevcEncoderConfig {
        profile: Some(HevcProfile::Main),
        ..Default::default()
    }),
    width: 1920,
    height: 1080,
    ..Default::default()
};
let mut encoder = Encoder::new(config)?;
```

## デコーダー API

### DecoderCodec enum

```rust
pub enum DecoderCodec {
    H264, Hevc, Av1, Vp8, Vp9, Jpeg,
}
```

### DecoderConfig

```rust
pub struct DecoderConfig {
    pub codec: DecoderCodec,
    pub device_id: i32,                    // デフォルト: 0
    pub max_num_decode_surfaces: u32,      // デフォルト: 20
    pub max_display_delay: u32,            // デフォルト: 0（低遅延）
}
```

### Decoder

```rust
impl Decoder {
    pub fn new(config: DecoderConfig) -> Result<Self, Error>;
    pub fn query_caps(codec: DecoderCodec, device_id: i32) -> Result<DecoderCaps, Error>;
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error>;
    pub fn finish(&mut self) -> Result<(), Error>;
    pub fn next_frame(&mut self) -> Result<Option<DecodedFrame>, Error>;
}
```

### 使用例

```rust
let config = DecoderConfig {
    codec: DecoderCodec::Av1,
    ..Default::default()
};
let mut decoder = Decoder::new(config)?;
```

## 参考

- WebCodecs API: `codec` 識別子 + コーデック固有 config を分離するパターン
- NVENC SDK: `NV_ENC_CODEC_CONFIG` union に `h264Config` / `hevcConfig` / `av1Config`

## 完了内容

以下の変更を実施した:

### エンコーダー

- `H264Profile`, `HevcProfile`, `Av1Profile` enum を追加（NVENC SDK 準拠）
- `H264EncoderConfig`, `HevcEncoderConfig`, `Av1EncoderConfig` 構造体を追加
- `CodecConfig` enum を追加（コーデック名とコーデック固有設定を一体化）
- `EncoderCodec` enum を追加（`query_caps` 用コーデック識別子）
- `EncoderConfig` から `profile` と `idr_period` を削除し、`codec: CodecConfig` を追加
- `Encoder::new(config)` に統一し、`new_h264` / `new_h265` / `new_av1` を削除
- `Encoder::query_caps(codec, device_id)` に統一し、`query_caps_h264` / `query_caps_h265` / `query_caps_av1` を削除
- `Profile` 構造体を削除

### デコーダー

- `DecoderCodec` enum を追加（デコーダー用コーデック識別子）
- `DecoderConfig` に `codec: DecoderCodec` フィールドを追加
- `Decoder::new(config)` に統一し、`new_h264` / `new_h265` / `new_av1` / `new_vp8` / `new_vp9` / `new_jpeg` を削除
- `Decoder::query_caps(codec, device_id)` に統一し、`query_caps_h264` / `query_caps_h265` / `query_caps_av1` / `query_caps_vp8` / `query_caps_vp9` / `query_caps_jpeg` を削除

### その他

- 全テストを新 API に書き換え
- `lib.rs` の pub use を更新
