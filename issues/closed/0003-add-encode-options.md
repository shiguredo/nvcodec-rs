# 0003 Encoder::encode() に EncodeOptions を追加する

## 概要

`Encoder::encode()` にフレーム単位のエンコードオプション `EncodeOptions` を追加し、キーフレーム強制やヘッダ出力の制御を可能にする。

## 背景

現在の `Encoder::encode()` は `NV_ENC_PIC_PARAMS` の `encodePicFlags` を設定しておらず、フレーム単位のオプション指定ができない。

NVENC SDK では `encodePicFlags` にビットフラグを指定することで、フレーム単位の制御が可能。

## 新しい API

```rust
/// NV_ENC_PIC_PARAMS の encodePicFlags に指定するオプション (NVENC: NV_ENC_PIC_FLAGS)
pub struct EncodeOptions {
    /// I フレームとして強制エンコードする (NVENC: NV_ENC_PIC_FLAG_FORCEINTRA)
    pub force_intra: bool,
    /// IDR フレームとして強制エンコードする (NVENC: NV_ENC_PIC_FLAG_FORCEIDR)
    /// AV1 の場合は Key Frame として扱われる
    pub force_idr: bool,
    /// SPS/PPS/VPS をビットストリームに出力する (NVENC: NV_ENC_PIC_FLAG_OUTPUT_SPSPPS)
    /// AV1 の場合は Sequence Header OBU が出力される
    pub output_spspps: bool,
}

impl Encoder {
    pub fn encode(
        &mut self,
        nv12_data: &[u8],
        options: &EncodeOptions,
    ) -> Result<(), Error> { ... }
}
```

## 設計方針

- `EncodeOptions` struct を新設する
- `Default` は実装しない
- `encode()` の引数に `options: &EncodeOptions` を追加する
- 各フラグが `true` の場合、`encodePicFlags` に対応するビットフラグをビット OR で設定する
- 全て `false` の場合、`encodePicFlags` は 0 のまま
- フィールド名は NVENC SDK のフラグ名に合わせる

## NVENC SDK の encodePicFlags (NV_ENC_PIC_FLAGS)

| フラグ | 値 | 説明 |
|--------|-----|------|
| `NV_ENC_PIC_FLAG_FORCEINTRA` | 0x1 | I フレームとして強制（非 IDR） |
| `NV_ENC_PIC_FLAG_FORCEIDR` | 0x2 | IDR フレームとして強制 |
| `NV_ENC_PIC_FLAG_OUTPUT_SPSPPS` | 0x4 | SPS/PPS をビットストリームに出力 |

## 変更対象ファイル

- `src/encode.rs`: `EncodeOptions` struct 追加、`encode()` の引数変更、`encode_picture()` で `encodePicFlags` を設定
- `src/lib.rs`: `EncodeOptions` を pub use に追加
- `build.rs`: スタブ定義に `NV_ENC_PIC_FLAG_FORCEINTRA` / `NV_ENC_PIC_FLAG_FORCEIDR` / `NV_ENC_PIC_FLAG_OUTPUT_SPSPPS` を追加
- `README.md`: エンコードセクションのコード例を更新
- `CHANGES.md`: 変更履歴を追加
