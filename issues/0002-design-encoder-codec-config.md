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
