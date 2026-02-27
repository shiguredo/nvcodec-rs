# 0002 エンコーダーのコーデック設定 API 設計

## 概要

現在の `Encoder::new_h264(EncoderConfig)` / `Encoder::new_h265(EncoderConfig)` は
コーデック名がメソッド名に埋め込まれており、`EncoderConfig` は全コーデック共通の
抽象化になっている。これは SDK の設計（コーデック別の `NV_ENC_CONFIG_H264` /
`NV_ENC_CONFIG_HEVC` / `NV_ENC_CONFIG_AV1`）と乖離している。

## 方針

`EncoderConfig` の `codec` フィールドに `CodecConfig` enum を持たせる。
enum のバリアントがコーデック名とコーデック固有 config を同時に表現する。

```rust
pub enum CodecConfig {
    H264(H264EncoderConfig),
    Hevc(HevcEncoderConfig),
    Av1(Av1EncoderConfig),
}

pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub bitrate: Option<u64>,
    // ...共通フィールド
    pub codec: CodecConfig,
}
```

コンストラクタは `Encoder::new(config)` に統一される。

```rust
let encoder = Encoder::new(EncoderConfig {
    width: 640,
    height: 480,
    codec: CodecConfig::H264(H264EncoderConfig {
        profile: H264Profile::Main,
        idr_period: 30,
    }),
    // ...
})?;
```

## 参考

- WebCodecs API: `codec` 識別子 + コーデック固有 config を分離するパターン
- NVENC SDK: `NV_ENC_CODEC_CONFIG` union に `h264Config` / `hevcConfig` / `av1Config`

## 低レイヤー / 高レイヤーの分離

この設計を低レイヤー API とし、使いやすい高レイヤー API をその上に提供する。

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
