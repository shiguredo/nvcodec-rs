---
name: shiguredo-nvcodec
description: 時雨堂の NVIDIA Video Codec SDK バインディング shiguredo_nvcodec の機能・API リファレンス。NVENC/NVCUVID によるハードウェアエンコード/デコード、コーデック情報照会、CUDA リソース管理、動的解像度変更に関する質問時に使用。
---

# shiguredo_nvcodec

[NVIDIA Video Codec SDK](https://developer.nvidia.com/video-codec-sdk) を利用したハードウェアビデオエンコーダー (NVENC) およびデコーダー (NVCUVID) の Rust バインディング。

## 特徴

- **NVENC**: ハードウェアエンコード (H.264 / HEVC / AV1)
- **NVCUVID**: ハードウェアデコード (H.264 / HEVC / AV1 / VP8 / VP9 / JPEG)
- **動的ロード**: CUDA ライブラリ (`libcuda.so.1` / `libnvcuvid.so.1` / `libnvidia-encode.so.1`) を `dlopen` で実行時にロード。ビルド時の CUDA Toolkit リンクは不要
- **ハンドラー型 API**: コンストラクタで [`EncodeHandler`] / [`DecodeHandler`] を渡し、ワーカースレッド上のコールバックで結果を受け取る
- **ケーパビリティ照会**: コーデックごとの最大解像度・対応プロファイル・対応機能をクエリ可能
- **動的解像度変更**: エンコーダーは [`reconfigure`] で明示変更、デコーダーはストリーム中の解像度変化を検出して再構成 / 再作成で対応
- **依存ゼロ (ランタイム)**: `tokio` / `async-std` 等の非同期ランタイム非依存

## バージョン情報

- crate 名: `shiguredo_nvcodec`
- バージョン: 2026.2.0
- Rust Edition: 2024
- 最小 Rust バージョン: 1.93
- ライセンス: Apache-2.0
- NVIDIA Video Codec SDK バージョン: 13.0.19

## 動作要件

- Linux (x86_64)
- NVIDIA GPU (Kepler 世代以降)
- NVIDIA ドライバー (CUDA ドライバー API を含む)
- ビルド時のみ: NVIDIA Video Codec SDK 13.0 以降のヘッダーファイル (`third_party/` 配下に配置)

docs.rs 向けには `DOCS_RS=1 cargo doc --no-deps` でスタブヘッダー経由のドキュメント生成のみ可能。

## コア API

### エンコード用

| 型 | 説明 | 主要メソッド・フィールド |
|----|------|------------------------|
| `Encoder<H: EncodeHandler>` | エンコーダー本体。内部で `nvcodec-encoder` / `nvcodec-drain` の 2 スレッドを起動 | `new(EncoderConfig, H)`, `encode(&[u8], &EncodeOptions, H::UserData)`, `flush()`, `reconfigure(ReconfigureParams)`, `get_sequence_params()`, `stats()` |
| `EncoderConfig` | エンコーダー設定 | `codec`, `width`, `height`, `max_encode_width`, `max_encode_height`, `framerate_num`, `framerate_den`, `average_bitrate`, `preset`, `tuning_info`, `rate_control_mode`, `gop_length`, `frame_interval_p`, `buffer_format`, `device_id` |
| `CodecConfig` | コーデック+プロファイル設定 enum | `H264(H264EncoderConfig)`, `Hevc(HevcEncoderConfig)`, `Av1(Av1EncoderConfig)` |
| `H264EncoderConfig` | H.264 固有設定 | `profile: Option<H264Profile>`, `idr_period: Option<u32>` |
| `HevcEncoderConfig` | HEVC 固有設定 | `profile: Option<HevcProfile>`, `idr_period: Option<u32>` |
| `Av1EncoderConfig` | AV1 固有設定 | `profile: Option<Av1Profile>`, `idr_period: Option<u32>` |
| `EncodeOptions` | フレーム単位のオプション | `force_intra: bool`, `force_idr: bool`, `output_spspps: bool` |
| `ReconfigureParams` | 動的再構成パラメータ (全フィールド `Option`) | `width`, `height`, `framerate_num`, `framerate_den`, `average_bitrate`, `max_bitrate` |
| `EncodedFrame<T>` | エンコード済みフレーム | `data()`, `timestamp()`, `picture_type()`, `user_data()`, `into_parts()` |
| `EncoderCaps` | エンコーダケーパビリティ | `supported_ratecontrol_modes`, `support_yuv444_encode`, `support_yuv422_encode`, `support_meonly_mode`, `width_max/min`, `height_max/min`, `num_max_bframes`, `support_10bit_encode`, `support_lossless_encode`, `support_lookahead`, `support_temporal_aq` |
| `EncoderStats` | エンコーダ統計値 (全フィールド `Counter`) | counter: `total_encoder_buffer_full_count` / gauge: `max_in_flight_frames` |
| `Counter` | 統計値のプリミティブ型 (通算値。`AtomicU64` の薄いラッパー。共有は `Arc<DecoderStats>` / `Arc<EncoderStats>` で行う。`inc()` は crate 内部のみ) | `new()`, `get() -> u64` |
| `Gauge` | 統計値のプリミティブ型 (時点値。`AtomicU64` の薄いラッパー。共有は `Arc<DecoderStats>` / `Arc<EncoderStats>` で行う。`set()` は crate 内部のみ) | `new()`, `get() -> u64` |

**プリセット定数** (`Preset`): `P1` (最高速) / `P2` / `P3` / `P4` (バランス) / `P5` / `P6` / `P7` (最高品質)

**チューニング情報定数** (`TuningInfo`): `HIGH_QUALITY` / `LOW_LATENCY` / `ULTRA_LOW_LATENCY` / `LOSSLESS`

**レート制御モード** (`RateControlMode`): `ConstQp` / `Vbr` / `Cbr` (`ConstQp` 以外は `average_bitrate` 必須)

**ピクチャータイプ** (`PictureType`): `P`, `B`, `I`, `Idr`, `Bi`, `Skipped`, `IntraRefresh`, `NonRefP`, `Switch`, `Unknown`

**プロファイル**:

| コーデック | プロファイル |
|-----------|-------------|
| `H264Profile` | `AutoSelect`, `Baseline`, `Main`, `High`, `High10`, `High422`, `High444`, `Stereo`, `ProgressiveHigh`, `ConstrainedHigh` |
| `HevcProfile` | `AutoSelect`, `Main`, `Main10`, `Frext` |
| `Av1Profile` | `AutoSelect`, `Main` |

### デコード用

| 型 | 説明 | 主要メソッド・フィールド |
|----|------|------------------------|
| `Decoder<H: DecodeHandler>` | デコーダー本体。内部で `nvcodec-decoder` ワーカースレッドを起動 | `new(DecoderConfig, H)`, `decode(&[u8], H::UserData)`, `flush()`, `stats()` |
| `DecoderConfig` | デコーダー設定 | `codec: DecoderCodec`, `device_id`, `max_num_decode_surfaces`, `max_display_delay`, `surface_format: SurfaceFormat`, `reconfigure_enabled: bool` |
| `DecoderCodec` | デコーダー対応コーデック | `H264`, `Hevc`, `Av1`, `Vp8`, `Vp9`, `Jpeg` |
| `SurfaceFormat` | 出力サーフェスフォーマット | `Nv12` のみ (他フォーマット要望時は `DecodedFrame` 拡張が必要) |
| `DecodedFrame<T>` | デコード済みフレーム (NV12) | `y_plane()`, `uv_plane()`, `y_stride()`, `uv_stride()`, `width()`, `height()`, `user_data()`, `into_parts()` |
| `DecoderCaps` | デコーダケーパビリティ | `is_supported`, `max_width`, `max_height`, `max_mb_count`, `min_width`, `min_height` |
| `DecoderStats` | デコーダ統計値 (全フィールド `Counter`) | counter: `total_create_decoder_count`, `total_reconfigure_decoder_count`, `total_reconfigure_failure_count`, `total_decode_count`, `total_sequence_callback_count`, `total_decode_callback_count`, `total_output_frame_count` |

#### デコードサーフェス数の決定

`DecoderConfig.max_num_decode_surfaces` はデコードサーフェス数の上限を指定する。`0` は指定できず、`Decoder::new` が設定エラーとして拒否する。NVDEC が正しいデコードに必要な最小サーフェス数 (`CUVIDEOFORMAT.min_num_decode_surfaces`) がこの上限を超える場合は、`decode()` 中の sequence callback で既存 decoder を破棄する前にエラーを返す。

実際に parser と decoder の両方へ適用する実効サーフェス数は、`min_num_decode_surfaces` と `max_num_decode_surfaces` から次の規則で決定する。実効サーフェス数は常に `max_num_decode_surfaces` 以下になる。

- `min_num_decode_surfaces == 0` は NVDEC からの不正な値としてエラーにする
- `min_num_decode_surfaces > max_num_decode_surfaces` は上限不足としてエラーにする
- `min_num_decode_surfaces >= 2` なら、その値を実効サーフェス数として使う
- `min_num_decode_surfaces == 1` かつ `max_num_decode_surfaces >= 2` なら、実効サーフェス数として 2 を使う
  - sequence callback の戻り値が 1 では parser の DPB 数を更新できないため、2 を使って parser と decoder のサーフェス数を一致させる
- `min_num_decode_surfaces == 1` かつ `max_num_decode_surfaces == 1` なら、実効サーフェス数として 1 を使う

codec 別の固定サーフェス数は使わない。`min_num_decode_surfaces` を超えるサーフェス数は性能とメモリ使用量の調整であり、正しさのための必須条件ではない。

### ハンドラートレイト

| トレイト | 説明 |
|---------|------|
| `EncodeHandler` | `type UserData: Send + 'static`, `type Error: From<Error> + Send + 'static`, `fn on_encoded(&mut self, Result<EncodedFrame<UserData>, Error>)` |
| `DecodeHandler` | `type UserData: Send + 'static`, `type Error: From<Error> + Send + 'static`, `fn on_decoded(&mut self, Result<DecodedFrame<UserData>, Error>)` |
| `FnEncodeHandler<T, E>` | `FnMut` クロージャを `EncodeHandler` にする薄いラッパー (`FnEncodeHandler::new(closure)`) |
| `FnDecodeHandler<T, E>` | `FnMut` クロージャを `DecodeHandler` にする薄いラッパー (`FnDecodeHandler::new(closure)`) |

ハンドラのメソッドはワーカースレッド上で呼ばれる。GUI スレッドやメインロジックへの戻しは `mpsc` などで自前で行う。

### コーデック情報照会

| 型・関数 | 説明 |
|---------|------|
| `supported_codecs(device_id) -> Result<Vec<CodecInfo>, Error>` | 指定 GPU で利用可能な全コーデックの情報を返す |
| `query_encoder_caps(EncoderCodec, device_id) -> Result<EncoderCaps, Error>` | 指定コーデックのエンコーダーケーパビリティ |
| `query_decoder_caps(DecoderCodec, device_id) -> Result<DecoderCaps, Error>` | 指定コーデックのデコーダーケーパビリティ |
| `VideoCodecType` | コーデック種別 enum (`H264`, `Hevc`, `Av1`, `Vp8`, `Vp9`, `Jpeg`) |
| `EncoderCodec` | エンコード対応コーデック (`H264`, `Hevc`, `Av1`) |
| `CodecInfo` | コーデックごとの情報 (`codec`, `decoding: DecodingInfo`, `encoding: EncodingInfo`) |
| `DecodingInfo` | デコード情報 (`supported`, `hardware_accelerated`, `max_width/height`, `min_width/height`, `max_mb_count`) |
| `EncodingInfo` | エンコード情報 (`supported`, `profiles: EncodingProfiles`, `max_width/height`, `min_width/height`, `num_max_bframes`, `supports_yuv444/yuv422/10bit/lossless/lookahead/temporal_aq`, `supported_ratecontrol_modes`) |
| `EncodingProfiles` | コーデック別プロファイル一覧 (`H264(Vec<H264EncodingProfile>)`, `Hevc(Vec<HevcEncodingProfile>)`, `Av1(Vec<Av1EncodingProfile>)`, `None`) |

`EncodingInfo::hardware_accelerated` と `DecodingInfo::hardware_accelerated` は NVENC / NVDEC が常にハードウェア処理であるため `supported` と同値。

### CUDA ユーティリティ

| 関数・型 | 説明 |
|---------|------|
| `is_cuda_library_available() -> bool` | `libcuda.so.1` がロード可能かを判定 (実際に CUDA が動作するかまでは確認しない) |
| `device_count() -> Result<i32, Error>` | CUDA デバイス数を取得 |
| `device_name(device_id: i32) -> Result<String, Error>` | 指定デバイス名を取得 |
| `mem_alloc_pitch(width_in_bytes, height, element_size_bytes) -> Result<(u64, usize), Error>` | アライメント要件を満たすピッチ付きデバイスメモリ確保。戻り値は `(device_ptr, pitch)` |
| `memcpy_2d(&Memcpy2DParams) -> Result<(), Error>` | ピッチが異なる 2D メモリ間のコピー |
| `Memcpy2DParams` | `src_memory_type`, `src_host`, `src_device`, `src_pitch`, `dst_memory_type`, `dst_host`, `dst_device`, `dst_pitch`, `width_in_bytes`, `height` |
| `MemoryType` | `Host` / `Device` |
| `CudaStream` | 非同期 GPU 操作用のストリーム。`new()`, `synchronize()`, `as_raw() -> CUstream`. `Drop` で破棄 |

### バージョン情報

| 定数 | 説明 |
|------|------|
| `BUILD_VERSION` | ビルド時に参照した NVIDIA Video Codec SDK のバージョン文字列 |

## エラー型

`Error` は CUDA / NVENC / クレート起因のエラーをまとめて表現する struct。`std::error::Error` と `Display` を実装。

| フィールド | 説明 |
|-----------|------|
| `function` | エラーが発生した関数名 (`&'static str`) |
| `status_code` | CUDA / NVENC のステータスコード (`Option<u32>`) |
| `status_name` | ステータスコードの名前 (`Option<Cow<'static, str>>`) |
| `status_message` | ステータスコードの説明 (`Option<Cow<'static, str>>`) |

`Display` の出力例:

- `encode() failed: invalid frame data size` (クレート起因)
- `cuMemAlloc_v2() failed[status=2]: out of memory (CUDA_ERROR_OUT_OF_MEMORY)` (CUDA エラー)
- `nvEncEncodePicture() failed[status=8]: One or more of the parameter passed to the API call is invalid (NV_ENC_ERR_INVALID_PARAM)` (NVENC エラー)

## サポートコーデック

### エンコード (NVENC)

| コーデック | `CodecConfig` |
|-----------|--------------|
| H.264 | `CodecConfig::H264(H264EncoderConfig)` |
| HEVC | `CodecConfig::Hevc(HevcEncoderConfig)` |
| AV1 | `CodecConfig::Av1(Av1EncoderConfig)` |

### デコード (NVCUVID)

| コーデック | `DecoderCodec` |
|-----------|--------------|
| H.264 | `DecoderCodec::H264` |
| HEVC | `DecoderCodec::Hevc` |
| AV1 | `DecoderCodec::Av1` |
| VP8 | `DecoderCodec::Vp8` |
| VP9 | `DecoderCodec::Vp9` |
| JPEG | `DecoderCodec::Jpeg` |

## サポートフォーマット

### エンコード入力 (`BufferFormat`)

| フォーマット | バリアント | 説明 |
|---|---|---|
| NV12 | `Nv12` | Semi-Planar YUV 4:2:0 8bit |
| YV12 | `Yv12` | Planar YUV 4:2:0 8bit (Y+V+U) |
| IYUV (I420) | `Iyuv` | Planar YUV 4:2:0 8bit (Y+U+V) |
| YUV444 | `Yuv444` | Planar YUV 4:4:4 8bit |
| YUV420 10bit | `Yuv420_10bit` | Semi-Planar YUV 4:2:0 10bit |
| YUV444 10bit | `Yuv444_10bit` | Planar YUV 4:4:4 10bit |
| ARGB | `Argb` | Packed A8R8G8B8 |
| ABGR | `Abgr` | Packed A8B8G8R8 |
| ARGB 10bit | `Argb10` | Packed A2R10G10B10 |
| ABGR 10bit | `Abgr10` | Packed A2B10G10R10 |

入力データのサイズは `width * height * bytes_per_pixel * subsampling_ratio` と一致している必要がある。不一致時は `Error: invalid frame data size`。

### デコード出力 (`SurfaceFormat`)

| フォーマット | バリアント | 説明 |
|---|---|---|
| NV12 | `Nv12` | Semi-Planar YUV 4:2:0 8bit |

現在のフレームコピー処理は NV12 前提のため、他フォーマットを追加する場合は `DecodedFrame` の拡張も同時に必要。

## コード例

### エンコーダー

```rust
use std::sync::mpsc;
use shiguredo_nvcodec::{
    BufferFormat, CodecConfig, EncodeOptions, EncodedFrame, Encoder, EncoderConfig, Error,
    FnEncodeHandler, H264EncoderConfig, Preset, RateControlMode, TuningInfo,
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

// ハンドラはワーカースレッド上で呼ばれるため、mpsc 経由でメインスレッドに渡す
let (tx, rx) = mpsc::sync_channel(4);
let encoder = Encoder::new(
    config,
    FnEncodeHandler::new(move |frame: Result<EncodedFrame<()>, Error>| {
        let _ = tx.send(frame);
    }),
)?;

// 通常のエンコード
let options = EncodeOptions {
    force_intra: false,
    force_idr: false,
    output_spspps: false,
};
encoder.encode(&nv12_data, &options, ())?;

// IDR 強制
let idr_options = EncodeOptions {
    force_intra: false,
    force_idr: true,
    output_spspps: false,
};
encoder.encode(&nv12_data, &idr_options, ())?;

// 全 in-flight フレームが完了するまで待機
encoder.flush()?;

// 結果を取得
for frame in rx.try_iter() {
    let frame = frame?;
    println!("encoded {} bytes, type={:?}", frame.data().len(), frame.picture_type());
}
```

### デコーダー

```rust
use std::sync::mpsc;
use shiguredo_nvcodec::{
    DecodedFrame, Decoder, DecoderCodec, DecoderConfig, Error, FnDecodeHandler, SurfaceFormat,
};

let config = DecoderConfig {
    codec: DecoderCodec::H264,
    device_id: 0,
    max_num_decode_surfaces: 20,
    max_display_delay: 0,
    reconfigure_enabled: false,
    surface_format: SurfaceFormat::Nv12,
};

let (tx, rx) = mpsc::sync_channel(4);
let decoder = Decoder::new(
    config,
    FnDecodeHandler::new(move |frame: Result<DecodedFrame<u64>, Error>| {
        let _ = tx.send(frame);
    }),
)?;

// Annex B 形式の H.264 を投入 (start code 0x00000001 を含む)
decoder.decode(&encoded_data, 0)?;
decoder.flush()?;

for frame in rx.try_iter() {
    let frame = frame?;
    let y = frame.y_plane();
    let uv = frame.uv_plane();
    let y_stride = frame.y_stride();
    let uv_stride = frame.uv_stride();
    println!(
        "decoded {}x{} (Y stride={}, UV stride={})",
        frame.width(),
        frame.height(),
        y_stride,
        uv_stride
    );
    let (data, user_data) = frame.into_parts();
    let _ = (data, user_data);
    let _ = (y, uv);
}
```

### コーデック情報の照会

```rust
use shiguredo_nvcodec::{supported_codecs, EncodingProfiles};

let codecs = supported_codecs(0)?;
for info in codecs {
    println!(
        "{:?}: decode={} encode={}",
        info.codec, info.decoding.supported, info.encoding.supported
    );
    if info.encoding.supported {
        println!(
            "  encode: {}x{} max, B-frames={}, 10bit={}, lossless={}",
            info.encoding.max_width,
            info.encoding.max_height,
            info.encoding.num_max_bframes,
            info.encoding.supports_10bit,
            info.encoding.supports_lossless
        );
        match info.encoding.profiles {
            EncodingProfiles::H264(ps) => println!("  H264 profiles: {:?}", ps),
            EncodingProfiles::Hevc(ps) => println!("  HEVC profiles: {:?}", ps),
            EncodingProfiles::Av1(ps) => println!("  AV1 profiles: {:?}", ps),
            EncodingProfiles::None => {}
        }
    }
}
```

### CUDA デバイス列挙

```rust
use shiguredo_nvcodec::{device_count, device_name, is_cuda_library_available};

if !is_cuda_library_available() {
    eprintln!("CUDA driver library not found");
    return Ok(());
}
let count = device_count()?;
for i in 0..count {
    println!("GPU {}: {}", i, device_name(i)?);
}
```

## 動的解像度変更

### エンコーダー

`reconfigure()` で解像度・ビットレート・フレームレートを動的に変更できる。エンコーダーの作り直しは不要。

| 制約 | 内容 |
|------|------|
| 上限 | 初期化時の `max_encode_width` / `max_encode_height` を超えてはならない (超えると `Error: width/height exceeds maxEncodeWidth/maxEncodeHeight`) |
| 解像度変更直後の最初のフレーム | `EncodeOptions { force_idr: true, output_spspps: true, .. }` を必ず指定すること。怠ると新解像度の SPS/PPS がビットストリームに出力されずデコーダー側で再生不能になる |

```rust
use shiguredo_nvcodec::ReconfigureParams;

// 作成時に最大解像度を確保
let config = EncoderConfig {
    width: 1920,
    height: 1080,
    max_encode_width: Some(3840),
    max_encode_height: Some(2160),
    // ...
};

// 解像度を変更
encoder.reconfigure(ReconfigureParams {
    width: Some(1280),
    height: Some(720),
    ..Default::default()
})?;

// 新解像度の最初のフレームは必ず IDR + SPS/PPS を出力
encoder.encode(&new_frame, &EncodeOptions {
    force_intra: false,
    force_idr: true,
    output_spspps: true,
}, ())?;
```

### デコーダー

ストリーム中に解像度が変わった場合、`DecoderConfig.reconfigure_enabled` で処理方式を選べる。最大解像度の指定は不要。

- `reconfigure_enabled: false` (推奨値) は、シーケンス変更ごとに decoder を破棄して再作成する。
- `reconfigure_enabled: true` は、現在の decoder session の上限 (作成時または再作成時の coded サイズ) 以内の解像度変化を `cuvidReconfigureDecoder` による in-place 再構成で処理し、上限を超える拡大やコーデック情報の変化は再作成で処理する。
- `reconfigure_enabled: true` は、解像度変更が頻繁に起こるストリームで、シーケンス変更ごとの decoder 再作成コスト (処理時間・遅延) を避けたい場合に指定する。一方、一度大きな解像度に達したあとに小さい解像度が長く続く場合は、decoder session の上限が大きなまま維持されるため、確保されるデコードサーフェスのメモリ消費の面では `reconfigure_enabled: false` の方が有利だ。
- `reconfigure_enabled: true` のとき、`cuvidReconfigureDecoder` が失敗した場合は decoder を破棄して再作成し、デコードを継続する (失敗回数は `DecoderStats::total_reconfigure_failure_count` で確認できる)。
- `reconfigure_enabled: true` は `max_display_delay > 0` と組み合わせられない (組み合わせた場合は `Decoder::new` が設定エラーを返す)。

`DecodedFrame` はフレームごとに `width()` / `height()` を持つので、フレームごとにサイズを確認する。

フレームの寸法と画素データは、そのフレームの表示領域 (display area) に一致する。`width()` / `height()` は表示領域の寸法を返す。画素データは行矩形ではなく各行が stride を持つバッファで、Y は `y_plane()[y * y_stride() + x]`、UV はインターリーブされたクロマを `uv_plane()` からアクセスする。`uv_stride()` は `width()` より大きいことがある。詳細な出力契約は `DecodedFrame` の rustdoc を参照する。

```rust
decoder.decode(&data_1080p, 0)?;
let frame = rx.recv()??;
assert_eq!(frame.width(), 1920);

decoder.decode(&data_720p, 1)?;
let frame = rx.recv()??;
assert_eq!(frame.width(), 1280);  // 自動的に追従
```

### まとめ

| | エンコーダー | デコーダー |
|---|---|---|
| 仕組み | `reconfigure()` で明示的に変更 | `reconfigure_enabled` に応じて再構成 / 再作成を使い分け |
| 利用者の操作 | `ReconfigureParams` で新解像度を指定 | `reconfigure_enabled` を指定するのみ (推奨値は false) |
| 制約 | `max_encode_width` / `max_encode_height` 以内 | session 上限 (作成時または再作成時の coded サイズ) 以内は再構成、超過は再作成 |
| 超えた場合 | エンコーダーを作り直す | 自動対応 |

## 統計値の取得

`Decoder::stats()` / `Encoder::stats()` で、デコーダー / エンコーダーの内部状態を統計値として取得できる。

統計値は `Counter` 型 (通算値) と `Gauge` 型 (時点値) で表現され、`get()` で現在値を読み出す。`stats()` が返すのは共有統計値への参照であり、`get()` を呼ぶたびに最新値が読める (値を保持したい場合は `clone()` でスナップショットを取得)。

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

デコーダーも同様に `decoder.stats()` で統計値を取得できる。

```rust
let stats = decoder.stats();
println!(
    "decode calls: {}, output frames: {}, in flight: {}",
    stats.total_decode_count.get(),
    stats.total_output_frame_count.get(),
    stats.in_flight_frames()
);
```

## スレッドモデル

| 構造体 | 内部スレッド | 役割 |
|--------|------------|------|
| `Encoder` | `nvcodec-encoder` (worker) | `Job` 受信、フレーム送信、バッファ管理 |
| `Encoder` | `nvcodec-drain` (drain) | `nvEncLockBitstream` のブロッキング呼び出し、結果取り出し |
| `Decoder` | `nvcodec-decoder` (worker) | `Job` 受信、`cuvidParseVideoData` 実行、デコード結果の取り出し |

- `encode()` / `decode()` は内部 mpsc に送るだけで即座に戻る (非同期)
- `flush()` は in-flight の全フレームが完了 (ハンドラ呼び出し) するまで同期的に待機
- `Drop` 時に worker / drain スレッドを終了させ、残フレームを drain してから戻る
- `Encoder<H>` / `Decoder<H>` は `Send` 実装で、フィールドがすべて `Sync` のため自動導出で `Sync` も成立する (`Arc` 共有で他スレッドから `stats()` を呼べる)

## 既知の制限事項

- **OS**: Linux (x86_64) のみ。Windows / macOS は未対応
- **GPU**: NVIDIA GPU 専用 (Kepler 世代以降)
- **デコード出力**: 現状 `SurfaceFormat::Nv12` のみ。他フォーマットを追加するには `DecodedFrame` のプレーン構造拡張とコピー処理の分岐が必要
- **エンコードコーデック**: VP8 / VP9 / JPEG は NVENC でエンコード非対応 (デコードのみ)
- **`bindgen` 自動変換の限界**: `#define` 定数や一部の `static` 変数は `build.rs` で直接定義しているため、SDK バージョン更新時は `build.rs` も併せて更新する必要がある
- **`third_party/` の更新**: NVIDIA Video Codec SDK のヘッダーは要ログインダウンロードのため `build.rs` でフェッチせず手動配置している。バージョン更新時は `Cargo.toml` の `[package.metadata.external-dependencies.nvcodec]` と `third_party/` の両方を更新する
- **`Decoder` の起動レイテンシ**: パーサーがシーケンスヘッダーを検出してからデコーダー作成 → 初回フレーム出力までは遅延がある。`max_display_delay = 0` でも完全な「1 フレーム入力 → 1 フレーム出力」にはならない
- **`Encoder::reconfigure` 後の SPS/PPS**: 解像度変更直後の最初のフレームは `force_idr: true` + `output_spspps: true` を指定しないと新解像度の SPS/PPS が欠落する (上記「動的解像度変更」参照)
