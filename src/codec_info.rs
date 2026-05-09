//! コーデック情報の照会

use crate::{CudaLibrary, DecoderCodec, EncoderCodec, Error};

/// コーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodecType {
    /// H.264 / AVC
    H264,
    /// H.265 / HEVC
    Hevc,
    /// AV1
    Av1,
    /// VP8
    Vp8,
    /// VP9
    Vp9,
    /// JPEG
    Jpeg,
}

impl VideoCodecType {
    /// すべてのコーデック種別を返す
    fn all() -> &'static [Self] {
        &[
            Self::H264,
            Self::Hevc,
            Self::Av1,
            Self::Vp8,
            Self::Vp9,
            Self::Jpeg,
        ]
    }

    /// DecoderCodec に変換する
    fn to_decoder_codec(self) -> DecoderCodec {
        match self {
            Self::H264 => DecoderCodec::H264,
            Self::Hevc => DecoderCodec::Hevc,
            Self::Av1 => DecoderCodec::Av1,
            Self::Vp8 => DecoderCodec::Vp8,
            Self::Vp9 => DecoderCodec::Vp9,
            Self::Jpeg => DecoderCodec::Jpeg,
        }
    }

    /// EncoderCodec に変換する
    ///
    /// VP8, VP9, JPEG は NVENC でエンコード非対応のため None を返す。
    fn to_encoder_codec(self) -> Option<EncoderCodec> {
        match self {
            Self::H264 => Some(EncoderCodec::H264),
            Self::Hevc => Some(EncoderCodec::Hevc),
            Self::Av1 => Some(EncoderCodec::Av1),
            Self::Vp8 | Self::Vp9 | Self::Jpeg => None,
        }
    }
}

/// コーデックごとの情報
#[derive(Debug, Clone)]
pub struct CodecInfo {
    /// コーデック種別
    pub codec: VideoCodecType,
    /// デコード情報
    pub decoding: DecodingInfo,
    /// エンコード情報
    pub encoding: EncodingInfo,
}

/// デコード情報
#[derive(Debug, Clone)]
pub struct DecodingInfo {
    /// デコードが可能か
    pub supported: bool,
    /// ハードウェアアクセラレーションが利用可能か
    ///
    /// NVDEC は常にハードウェアアクセラレーションであるため、
    /// supported と同じ値になる。
    pub hardware_accelerated: bool,
    /// 最大デコード幅
    pub max_width: u32,
    /// 最大デコード高さ
    pub max_height: u32,
    /// 最小デコード幅
    pub min_width: u32,
    /// 最小デコード高さ
    pub min_height: u32,
    /// 最大マクロブロック数
    pub max_mb_count: u32,
}

/// エンコード情報
#[derive(Debug, Clone)]
pub struct EncodingInfo {
    /// エンコードが可能か
    pub supported: bool,
    /// ハードウェアアクセラレーションが利用可能か
    ///
    /// NVENC は常にハードウェアアクセラレーションであるため、
    /// supported と同じ値になる。
    pub hardware_accelerated: bool,
    /// コーデック固有のプロファイル情報
    pub profiles: EncodingProfiles,
    /// 最大エンコード幅
    pub max_width: i32,
    /// 最大エンコード高さ
    pub max_height: i32,
    /// 最小エンコード幅
    pub min_width: i32,
    /// 最小エンコード高さ
    pub min_height: i32,
    /// B フレームの最大数
    pub num_max_bframes: i32,
    /// YUV444 エンコードをサポートするか
    pub supports_yuv444: bool,
    /// YUV422 エンコードをサポートするか
    pub supports_yuv422: bool,
    /// 10bit エンコードをサポートするか
    pub supports_10bit: bool,
    /// ロスレスエンコードをサポートするか
    pub supports_lossless: bool,
    /// 先読みエンコードをサポートするか
    pub supports_lookahead: bool,
    /// Temporal AQ をサポートするか
    pub supports_temporal_aq: bool,
    /// サポートされているレート制御モードのビットマスク
    pub supported_ratecontrol_modes: i32,
}

impl EncodingInfo {
    /// エンコード非対応の場合の値を返す
    fn unsupported() -> Self {
        Self {
            supported: false,
            hardware_accelerated: false,
            profiles: EncodingProfiles::None,
            max_width: 0,
            max_height: 0,
            min_width: 0,
            min_height: 0,
            num_max_bframes: 0,
            supports_yuv444: false,
            supports_yuv422: false,
            supports_10bit: false,
            supports_lossless: false,
            supports_lookahead: false,
            supports_temporal_aq: false,
            supported_ratecontrol_modes: 0,
        }
    }
}

/// コーデック固有のエンコードプロファイル情報
///
/// NVENC SDK で定義されているプロファイル GUID に基づく静的な列挙。
/// 動的なプロファイル照会 API (nvEncGetEncodeProfileGUIDs) は
/// 現在の実装では使用していない。
#[derive(Debug, Clone, PartialEq)]
pub enum EncodingProfiles {
    /// H.264 プロファイル一覧
    H264(Vec<H264EncodingProfile>),
    /// HEVC プロファイル一覧
    Hevc(Vec<HevcEncodingProfile>),
    /// AV1 プロファイル一覧
    Av1(Vec<Av1EncodingProfile>),
    /// プロファイル情報なし（エンコード非対応）
    None,
}

/// H.264 エンコードプロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264EncodingProfile {
    /// Baseline
    Baseline,
    /// Main
    Main,
    /// High
    High,
    /// High 10
    High10,
    /// High 4:2:2
    High422,
    /// High 4:4:4
    High444,
    /// Stereo
    Stereo,
    /// Progressive High
    ProgressiveHigh,
    /// Constrained High
    ConstrainedHigh,
}

/// HEVC エンコードプロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcEncodingProfile {
    /// Main
    Main,
    /// Main10
    Main10,
    /// Frext (Format Range Extensions)
    Frext,
}

/// AV1 エンコードプロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1EncodingProfile {
    /// Main
    Main,
}

/// 指定 GPU デバイスで利用可能なコーデック情報の一覧を返す
///
/// CUDA ライブラリのロードに失敗した場合はエラーを返す。
/// 個別コーデックのケーパビリティ取得に失敗した場合は、
/// そのコーデックを非対応として扱う。
pub fn supported_codecs(device_id: i32) -> Result<Vec<CodecInfo>, Error> {
    // CUDA ライブラリのロードを先に行い、失敗したらエラーを返す
    let _lib = CudaLibrary::load()?;

    let codecs = VideoCodecType::all()
        .iter()
        .map(|&codec| CodecInfo {
            codec,
            decoding: probe_decoding(codec, device_id),
            encoding: probe_encoding(codec, device_id),
        })
        .collect();

    Ok(codecs)
}

/// NVDEC のデコードケーパビリティを照会する
fn probe_decoding(codec: VideoCodecType, device_id: i32) -> DecodingInfo {
    let decoder_codec = codec.to_decoder_codec();
    match crate::query_decoder_caps(decoder_codec, device_id) {
        Ok(caps) => DecodingInfo {
            supported: caps.is_supported,
            hardware_accelerated: caps.is_supported,
            max_width: caps.max_width,
            max_height: caps.max_height,
            min_width: caps.min_width,
            min_height: caps.min_height,
            max_mb_count: caps.max_mb_count,
        },
        Err(_) => DecodingInfo {
            supported: false,
            hardware_accelerated: false,
            max_width: 0,
            max_height: 0,
            min_width: 0,
            min_height: 0,
            max_mb_count: 0,
        },
    }
}

/// NVENC のエンコードケーパビリティを照会する
fn probe_encoding(codec: VideoCodecType, device_id: i32) -> EncodingInfo {
    let encoder_codec = match codec.to_encoder_codec() {
        Some(c) => c,
        None => return EncodingInfo::unsupported(),
    };

    match crate::query_encoder_caps(encoder_codec, device_id) {
        Ok(caps) => {
            let supported = caps.width_max > 0;
            EncodingInfo {
                supported,
                hardware_accelerated: supported,
                profiles: if supported {
                    profiles_for_codec(codec)
                } else {
                    EncodingProfiles::None
                },
                max_width: caps.width_max,
                max_height: caps.height_max,
                min_width: caps.width_min,
                min_height: caps.height_min,
                num_max_bframes: caps.num_max_bframes,
                supports_yuv444: caps.support_yuv444_encode,
                supports_yuv422: caps.support_yuv422_encode,
                supports_10bit: caps.support_10bit_encode,
                supports_lossless: caps.support_lossless_encode,
                supports_lookahead: caps.support_lookahead,
                supports_temporal_aq: caps.support_temporal_aq,
                supported_ratecontrol_modes: caps.supported_ratecontrol_modes,
            }
        }
        Err(_) => EncodingInfo::unsupported(),
    }
}

/// NVENC SDK で定義されているプロファイルを静的に列挙する
fn profiles_for_codec(codec: VideoCodecType) -> EncodingProfiles {
    match codec {
        VideoCodecType::H264 => EncodingProfiles::H264(vec![
            H264EncodingProfile::Baseline,
            H264EncodingProfile::Main,
            H264EncodingProfile::High,
            H264EncodingProfile::High10,
            H264EncodingProfile::High422,
            H264EncodingProfile::High444,
            H264EncodingProfile::Stereo,
            H264EncodingProfile::ProgressiveHigh,
            H264EncodingProfile::ConstrainedHigh,
        ]),
        VideoCodecType::Hevc => EncodingProfiles::Hevc(vec![
            HevcEncodingProfile::Main,
            HevcEncodingProfile::Main10,
            HevcEncodingProfile::Frext,
        ]),
        VideoCodecType::Av1 => EncodingProfiles::Av1(vec![Av1EncodingProfile::Main]),
        _ => EncodingProfiles::None,
    }
}
