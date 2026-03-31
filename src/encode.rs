use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;

use crate::{CudaLibrary, Error, ReleaseGuard, sys};

/// プリセット
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset(sys::GUID);

impl Preset {
    /// P1プリセット（最高速）
    pub const P1: Self = Self(sys::NV_ENC_PRESET_P1_GUID);

    /// P2プリセット
    pub const P2: Self = Self(sys::NV_ENC_PRESET_P2_GUID);

    /// P3プリセット
    pub const P3: Self = Self(sys::NV_ENC_PRESET_P3_GUID);

    /// P4プリセット（バランス型）
    pub const P4: Self = Self(sys::NV_ENC_PRESET_P4_GUID);

    /// P5プリセット
    pub const P5: Self = Self(sys::NV_ENC_PRESET_P5_GUID);

    /// P6プリセット
    pub const P6: Self = Self(sys::NV_ENC_PRESET_P6_GUID);

    /// P7プリセット（最高品質）
    pub const P7: Self = Self(sys::NV_ENC_PRESET_P7_GUID);

    fn to_sys(self) -> sys::GUID {
        self.0
    }
}

/// チューニング情報
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuningInfo(sys::NV_ENC_TUNING_INFO);

impl TuningInfo {
    /// 高品質
    pub const HIGH_QUALITY: Self = Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY);

    /// 低遅延
    pub const LOW_LATENCY: Self = Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOW_LATENCY);

    /// 超低遅延
    pub const ULTRA_LOW_LATENCY: Self =
        Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY);

    /// ロスレス
    pub const LOSSLESS: Self = Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOSSLESS);

    fn to_sys(self) -> sys::NV_ENC_TUNING_INFO {
        self.0
    }
}

/// H.264 プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    /// 自動選択
    AutoSelect,
    /// Baseline プロファイル
    Baseline,
    /// Main プロファイル
    Main,
    /// High プロファイル
    High,
    /// High 10 プロファイル
    High10,
    /// High 4:2:2 プロファイル
    High422,
    /// High 4:4:4 プロファイル
    High444,
    /// Stereo プロファイル
    Stereo,
    /// Progressive High プロファイル
    ProgressiveHigh,
    /// Constrained High プロファイル
    ConstrainedHigh,
}

impl H264Profile {
    fn to_sys(self) -> sys::GUID {
        match self {
            H264Profile::AutoSelect => sys::NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID,
            H264Profile::Baseline => sys::NV_ENC_H264_PROFILE_BASELINE_GUID,
            H264Profile::Main => sys::NV_ENC_H264_PROFILE_MAIN_GUID,
            H264Profile::High => sys::NV_ENC_H264_PROFILE_HIGH_GUID,
            H264Profile::High10 => sys::NV_ENC_H264_PROFILE_HIGH_10_GUID,
            H264Profile::High422 => sys::NV_ENC_H264_PROFILE_HIGH_422_GUID,
            H264Profile::High444 => sys::NV_ENC_H264_PROFILE_HIGH_444_GUID,
            H264Profile::Stereo => sys::NV_ENC_H264_PROFILE_STEREO_GUID,
            H264Profile::ProgressiveHigh => sys::NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID,
            H264Profile::ConstrainedHigh => sys::NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID,
        }
    }
}

/// HEVC プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcProfile {
    /// 自動選択
    AutoSelect,
    /// Main プロファイル
    Main,
    /// Main 10 プロファイル
    Main10,
    /// Main 4:2:2/4:4:4 8/10 bit プロファイル
    Frext,
}

impl HevcProfile {
    fn to_sys(self) -> sys::GUID {
        match self {
            HevcProfile::AutoSelect => sys::NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID,
            HevcProfile::Main => sys::NV_ENC_HEVC_PROFILE_MAIN_GUID,
            HevcProfile::Main10 => sys::NV_ENC_HEVC_PROFILE_MAIN10_GUID,
            HevcProfile::Frext => sys::NV_ENC_HEVC_PROFILE_FREXT_GUID,
        }
    }
}

/// AV1 プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1Profile {
    /// 自動選択
    AutoSelect,
    /// Main プロファイル
    Main,
}

impl Av1Profile {
    fn to_sys(self) -> sys::GUID {
        match self {
            Av1Profile::AutoSelect => sys::NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID,
            Av1Profile::Main => sys::NV_ENC_AV1_PROFILE_MAIN_GUID,
        }
    }
}

/// H.264 エンコーダー固有設定 (NVENC: NV_ENC_CONFIG_H264)
#[derive(Debug, Clone)]
pub struct H264EncoderConfig {
    /// プロファイル (NVENC: profileGUID)
    /// None の場合は Main
    pub profile: Option<H264Profile>,
    /// IDR フレーム間隔 (NVENC: idrPeriod)
    /// None の場合は gop_length と同じ
    pub idr_period: Option<u32>,
}

/// HEVC エンコーダー固有設定 (NVENC: NV_ENC_CONFIG_HEVC)
#[derive(Debug, Clone)]
pub struct HevcEncoderConfig {
    /// プロファイル (NVENC: profileGUID)
    /// None の場合は Main
    pub profile: Option<HevcProfile>,
    /// IDR フレーム間隔 (NVENC: idrPeriod)
    /// None の場合は gop_length と同じ
    pub idr_period: Option<u32>,
}

/// AV1 エンコーダー固有設定 (NVENC: NV_ENC_CONFIG_AV1)
#[derive(Debug, Clone)]
pub struct Av1EncoderConfig {
    /// プロファイル (NVENC: profileGUID)
    /// None の場合は Main
    pub profile: Option<Av1Profile>,
    /// IDR フレーム間隔 (NVENC: idrPeriod)
    /// None の場合は gop_length と同じ
    pub idr_period: Option<u32>,
}

/// コーデックとコーデック固有設定
#[derive(Debug, Clone)]
pub enum CodecConfig {
    /// H.264
    H264(H264EncoderConfig),
    /// HEVC
    Hevc(HevcEncoderConfig),
    /// AV1
    Av1(Av1EncoderConfig),
}

/// エンコーダー用コーデック識別子（query_caps 用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderCodec {
    /// H.264
    H264,
    /// HEVC
    Hevc,
    /// AV1
    Av1,
}

/// エンコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// コーデックとコーデック固有設定 (NVENC: encodeGUID + NV_ENC_CODEC_CONFIG)
    pub codec: CodecConfig,

    /// エンコード幅 (NVENC: encodeWidth)
    pub width: u32,

    /// エンコード高さ (NVENC: encodeHeight)
    pub height: u32,

    /// 最大エンコード幅 (NVENC: maxEncodeWidth)
    /// None の場合は width と同じ値が使用される
    pub max_encode_width: Option<u32>,

    /// 最大エンコード高さ (NVENC: maxEncodeHeight)
    /// None の場合は height と同じ値が使用される
    pub max_encode_height: Option<u32>,

    /// フレームレートの分子 (NVENC: frameRateNum)
    pub framerate_num: u32,

    /// フレームレートの分母 (NVENC: frameRateDen)
    pub framerate_den: u32,

    /// 平均ビットレート (bps 単位, NVENC: averageBitRate)
    /// None の場合はレート制御モードが ConstQp である必要がある
    pub average_bitrate: Option<u32>,

    /// プリセット (NVENC: presetGUID)
    pub preset: Preset,

    /// チューニング情報 (NVENC: tuningInfo)
    pub tuning_info: TuningInfo,

    /// レート制御モード (NVENC: rateControlMode)
    pub rate_control_mode: RateControlMode,

    /// GOP 長 (NVENC: gopLength)
    /// None の場合は無限 GOP (NVENC_INFINITE_GOPLENGTH) が使用される
    pub gop_length: Option<u32>,

    /// P フレーム間隔 (NVENC: frameIntervalP)
    pub frame_interval_p: u32,

    /// 入力バッファフォーマット (NVENC: bufferFormat)
    pub buffer_format: BufferFormat,

    /// デバイス ID (使用する GPU)
    pub device_id: i32,
}

/// レート制御モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateControlMode {
    /// Constant QP mode
    ConstQp,

    /// Variable bitrate mode
    Vbr,

    /// Constant bitrate mode
    Cbr,
}

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

impl BufferFormat {
    fn to_sys(self) -> sys::NV_ENC_BUFFER_FORMAT {
        match self {
            BufferFormat::Nv12 => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_NV12,
            BufferFormat::Yv12 => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YV12,
            BufferFormat::Iyuv => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_IYUV,
            BufferFormat::Yuv444 => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YUV444,
            BufferFormat::Yuv420_10bit => {
                sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YUV420_10BIT
            }
            BufferFormat::Yuv444_10bit => {
                sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YUV444_10BIT
            }
            BufferFormat::Argb => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ARGB,
            BufferFormat::Abgr => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ABGR,
            BufferFormat::Argb10 => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ARGB10,
            BufferFormat::Abgr10 => sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ABGR10,
        }
    }

    /// Y プレーン (または packed フォーマット) の 1 行あたりのバイト数を返す
    fn bytes_per_row(self, width: u32) -> u32 {
        match self {
            // Planar 8bit: 1 byte/pixel
            BufferFormat::Nv12 | BufferFormat::Yv12 | BufferFormat::Iyuv | BufferFormat::Yuv444 => {
                width
            }
            // Planar 10bit: 2 bytes/pixel
            BufferFormat::Yuv420_10bit | BufferFormat::Yuv444_10bit => width * 2,
            // Packed 8bit: 4 bytes/pixel (ARGB/ABGR)
            BufferFormat::Argb | BufferFormat::Abgr => width * 4,
            // Packed 10bit: 4 bytes/pixel (A2R10G10B10/A2B10G10R10)
            BufferFormat::Argb10 | BufferFormat::Abgr10 => width * 4,
        }
    }

    /// 指定された幅と高さに対するフレームデータのバイトサイズを計算する
    fn frame_size(self, width: u32, height: u32) -> usize {
        let pixels = (width * height) as usize;
        match self {
            // YUV 4:2:0 (8bit): width * height * 3 / 2
            BufferFormat::Nv12 | BufferFormat::Yv12 | BufferFormat::Iyuv => pixels * 3 / 2,
            // YUV 4:4:4 (8bit): width * height * 3
            BufferFormat::Yuv444 => pixels * 3,
            // YUV 4:2:0 (10bit, 2 bytes/pixel): width * height * 3
            BufferFormat::Yuv420_10bit => pixels * 3,
            // YUV 4:4:4 (10bit, 2 bytes/pixel): width * height * 6
            BufferFormat::Yuv444_10bit => pixels * 6,
            // Packed (8bit, 4 bytes/pixel): width * height * 4
            BufferFormat::Argb | BufferFormat::Abgr => pixels * 4,
            // Packed (10bit, 4 bytes/pixel): width * height * 4
            BufferFormat::Argb10 | BufferFormat::Abgr10 => pixels * 4,
        }
    }
}

impl RateControlMode {
    fn to_sys(self) -> sys::NV_ENC_PARAMS_RC_MODE {
        match self {
            RateControlMode::ConstQp => sys::_NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_CONSTQP,
            RateControlMode::Vbr => sys::_NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_VBR,
            RateControlMode::Cbr => sys::_NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_CBR,
        }
    }
}

/// エンコーダのケーパビリティ情報
#[derive(Debug, Clone)]
pub struct EncoderCaps {
    /// サポートされているレート制御モードのビットマスク
    pub supported_ratecontrol_modes: i32,
    /// YUV444 エンコードのサポート
    pub support_yuv444_encode: bool,
    /// YUV422 エンコードのサポート
    pub support_yuv422_encode: bool,
    /// ME-only モードのサポート
    pub support_meonly_mode: bool,
    /// 最大エンコード幅
    pub width_max: i32,
    /// 最大エンコード高さ
    pub height_max: i32,
    /// 最小エンコード幅
    pub width_min: i32,
    /// 最小エンコード高さ
    pub height_min: i32,
    /// B フレームの最大数
    pub num_max_bframes: i32,
    /// 10bit エンコードのサポート
    pub support_10bit_encode: bool,
    /// ロスレスエンコードのサポート
    pub support_lossless_encode: bool,
    /// 先読みエンコードのサポート
    pub support_lookahead: bool,
    /// Temporal AQ のサポート
    pub support_temporal_aq: bool,
}

/// エンコーダ再構成パラメータ
#[derive(Debug, Clone, Default)]
pub struct ReconfigureParams {
    /// エンコード幅 (NVENC: encodeWidth)
    /// maxEncodeWidth を超えてはならない
    pub width: Option<u32>,
    /// エンコード高さ (NVENC: encodeHeight)
    /// maxEncodeHeight を超えてはならない
    pub height: Option<u32>,
    /// フレームレートの分子 (NVENC: frameRateNum)
    pub framerate_num: Option<u32>,
    /// フレームレートの分母 (NVENC: frameRateDen)
    pub framerate_den: Option<u32>,
    /// 平均ビットレート (bps, NVENC: averageBitRate)
    pub average_bitrate: Option<u32>,
    /// 最大ビットレート (bps, NVENC: maxBitRate)
    pub max_bitrate: Option<u32>,
}

/// フレーム単位のエンコードオプション (NVENC: NV_ENC_PIC_FLAGS)
#[derive(Debug, Clone)]
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

impl EncodeOptions {
    /// encodePicFlags のビットフラグに変換する
    fn to_pic_flags(&self) -> u32 {
        let mut flags = 0u32;
        if self.force_intra {
            flags |= sys::NV_ENC_PIC_FLAG_FORCEINTRA;
        }
        if self.force_idr {
            flags |= sys::NV_ENC_PIC_FLAG_FORCEIDR;
        }
        if self.output_spspps {
            flags |= sys::NV_ENC_PIC_FLAG_OUTPUT_SPSPPS;
        }
        flags
    }
}

/// エンコーダー
pub struct Encoder {
    lib: CudaLibrary,
    ctx: sys::CUcontext,
    encoder: sys::NV_ENCODE_API_FUNCTION_LIST,
    h_encoder: *mut c_void,
    width: u32,
    height: u32,
    buffer_format: sys::NV_ENC_BUFFER_FORMAT,
    buffer_format_enum: BufferFormat,
    expected_frame_size: usize,
    encoded_frames: VecDeque<EncodedFrame>,
    framerate_den: u64,
    frame_count: u64,
    init_params: sys::NV_ENC_INITIALIZE_PARAMS,
    encode_config: sys::NV_ENC_CONFIG,
}

impl Encoder {
    /// 指定されたコーデック設定でエンコーダーインスタンスを生成する
    pub fn new(config: EncoderConfig) -> Result<Self, Error> {
        unsafe {
            let lib = CudaLibrary::load()?;

            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let ctx_flags = 0; // デフォルトのコンテキストフラグ
            lib.cu_ctx_create(&mut ctx, ctx_flags, config.device_id)?;

            let lib_clone = lib.clone();
            let ctx_guard = ReleaseGuard::new(move || {
                let _ = lib_clone.cu_ctx_destroy(ctx);
            });

            // NVENC 操作のために CUDA context をアクティブ化し、エンコードセッションを開く
            let (encoder_api, h_encoder) = lib.with_context(ctx, || {
                // NVENC API をロード
                let mut encoder_api: sys::NV_ENCODE_API_FUNCTION_LIST = std::mem::zeroed();
                encoder_api.version = sys::NV_ENCODE_API_FUNCTION_LIST_VER;

                lib.nvenc_create_api_instance(&mut encoder_api)?;

                // エンコードセッションを開く
                let mut open_session_params: sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS =
                    std::mem::zeroed();
                open_session_params.version = sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
                open_session_params.deviceType = sys::_NV_ENC_DEVICE_TYPE_NV_ENC_DEVICE_TYPE_CUDA;
                open_session_params.device = ctx.cast();
                open_session_params.apiVersion = sys::NVENCAPI_VERSION;

                let mut h_encoder = ptr::null_mut();
                let status = encoder_api
                    .nvEncOpenEncodeSessionEx
                    .map(|f| f(&mut open_session_params, &mut h_encoder))
                    .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
                Error::check_nvenc(status, "nvEncOpenEncodeSessionEx")?;

                Ok((encoder_api, h_encoder))
            })?;

            // ここまで成功したらクリーンアップをキャンセル（あとは Drop に任せる）
            ctx_guard.cancel();

            let mut encoder = Self {
                lib: lib.clone(),
                ctx,
                encoder: encoder_api,
                h_encoder,
                width: config.width,
                height: config.height,
                buffer_format: config.buffer_format.to_sys(),
                buffer_format_enum: config.buffer_format,
                expected_frame_size: config.buffer_format.frame_size(config.width, config.height),
                encoded_frames: VecDeque::new(),
                framerate_den: config.framerate_den as u64,
                frame_count: 0,
                init_params: std::mem::zeroed(),
                encode_config: std::mem::zeroed(),
            };

            // デフォルトパラメータでエンコーダーを初期化
            lib.with_context(ctx, || encoder.initialize_encoder(&config))?;

            Ok(encoder)
        }
    }

    /// 指定コーデックのエンコーダのケーパビリティをクエリする
    pub fn query_caps(codec: EncoderCodec, device_id: i32) -> Result<EncoderCaps, Error> {
        let codec_guid = match codec {
            EncoderCodec::H264 => sys::NV_ENC_CODEC_H264_GUID,
            EncoderCodec::Hevc => sys::NV_ENC_CODEC_HEVC_GUID,
            EncoderCodec::Av1 => sys::NV_ENC_CODEC_AV1_GUID,
        };
        Self::query_caps_with_codec(device_id, codec_guid)
    }

    /// 指定コーデックのエンコーダのケーパビリティをクエリする
    fn query_caps_with_codec(device_id: i32, codec_guid: sys::GUID) -> Result<EncoderCaps, Error> {
        unsafe {
            let lib = CudaLibrary::load()?;

            // 一時的な CUDA コンテキストを作成
            let mut ctx = ptr::null_mut();
            lib.cu_ctx_create(&mut ctx, 0, device_id)?;

            let lib_clone = lib.clone();
            let ctx_guard = ReleaseGuard::new(move || {
                let _ = lib_clone.cu_ctx_destroy(ctx);
            });

            let caps = lib.with_context(ctx, || {
                // NVENC API をロード
                let mut encoder_api: sys::NV_ENCODE_API_FUNCTION_LIST = std::mem::zeroed();
                encoder_api.version = sys::NV_ENCODE_API_FUNCTION_LIST_VER;
                lib.nvenc_create_api_instance(&mut encoder_api)?;

                // エンコードセッションを開く
                let mut open_session_params: sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS =
                    std::mem::zeroed();
                open_session_params.version = sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
                open_session_params.deviceType = sys::_NV_ENC_DEVICE_TYPE_NV_ENC_DEVICE_TYPE_CUDA;
                open_session_params.device = ctx.cast();
                open_session_params.apiVersion = sys::NVENCAPI_VERSION;

                let mut h_encoder = ptr::null_mut();
                let status = encoder_api
                    .nvEncOpenEncodeSessionEx
                    .map(|f| f(&mut open_session_params, &mut h_encoder))
                    .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
                Error::check_nvenc(status, "nvEncOpenEncodeSessionEx")?;

                // セッションを確実に閉じるためのガード
                let destroy_fn = encoder_api.nvEncDestroyEncoder;
                let session_guard = ReleaseGuard::new(move || {
                    if let Some(f) = destroy_fn {
                        f(h_encoder);
                    }
                });

                // 各ケーパビリティ値をクエリするヘルパー
                let query_cap = |caps_type: u32| -> Result<i32, Error> {
                    let mut caps_param: sys::NV_ENC_CAPS_PARAM = std::mem::zeroed();
                    caps_param.version = sys::NV_ENC_CAPS_PARAM_VER;
                    caps_param.capsToQuery = caps_type;

                    let mut caps_val: i32 = 0;
                    let status = encoder_api
                        .nvEncGetEncodeCaps
                        .map(|f| f(h_encoder, codec_guid, &mut caps_param, &mut caps_val))
                        .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
                    Error::check_nvenc(status, "nvEncGetEncodeCaps")?;
                    Ok(caps_val)
                };

                let caps = EncoderCaps {
                    supported_ratecontrol_modes: query_cap(
                        sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORTED_RATECONTROL_MODES,
                    )?,
                    support_yuv444_encode: query_cap(
                        sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_YUV444_ENCODE,
                    )? != 0,
                    support_yuv422_encode: query_cap(
                        sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_YUV422_ENCODE,
                    )? != 0,
                    support_meonly_mode: query_cap(
                        sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_MEONLY_MODE,
                    )? != 0,
                    width_max: query_cap(sys::_NV_ENC_CAPS_NV_ENC_CAPS_WIDTH_MAX)?,
                    height_max: query_cap(sys::_NV_ENC_CAPS_NV_ENC_CAPS_HEIGHT_MAX)?,
                    width_min: query_cap(sys::_NV_ENC_CAPS_NV_ENC_CAPS_WIDTH_MIN)?,
                    height_min: query_cap(sys::_NV_ENC_CAPS_NV_ENC_CAPS_HEIGHT_MIN)?,
                    num_max_bframes: query_cap(sys::_NV_ENC_CAPS_NV_ENC_CAPS_NUM_MAX_BFRAMES)?,
                    support_10bit_encode: query_cap(
                        sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_10BIT_ENCODE,
                    )? != 0,
                    support_lossless_encode: query_cap(
                        sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_LOSSLESS_ENCODE,
                    )? != 0,
                    support_lookahead: query_cap(sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_LOOKAHEAD)?
                        != 0,
                    support_temporal_aq: query_cap(
                        sys::_NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_TEMPORAL_AQ,
                    )? != 0,
                };

                // セッションガードがスコープアウト時にエンコーダを破棄する
                drop(session_guard);

                Ok(caps)
            })?;

            ctx_guard.cancel();
            lib.cu_ctx_destroy(ctx)?;

            Ok(caps)
        }
    }

    /// エンコーダパラメータを再構成する
    ///
    /// ビットレートやフレームレートを動的に変更する。
    /// エンコーダの初期化時に設定された値を基準に、指定されたパラメータのみを上書きする。
    pub fn reconfigure(&mut self, params: ReconfigureParams) -> Result<(), Error> {
        self.lib
            .clone()
            .with_context(self.ctx, || self.reconfigure_inner(params))
    }

    fn reconfigure_inner(&mut self, params: ReconfigureParams) -> Result<(), Error> {
        unsafe {
            // 解像度が maxEncodeWidth / maxEncodeHeight を超えないか検証する
            if let Some(width) = params.width
                && width > self.init_params.maxEncodeWidth
            {
                return Err(Error::new_custom(
                    "reconfigure",
                    "width exceeds maxEncodeWidth",
                ));
            }
            if let Some(height) = params.height
                && height > self.init_params.maxEncodeHeight
            {
                return Err(Error::new_custom(
                    "reconfigure",
                    "height exceeds maxEncodeHeight",
                ));
            }

            let mut reconfig_params: sys::NV_ENC_RECONFIGURE_PARAMS = std::mem::zeroed();
            reconfig_params.version = sys::NV_ENC_RECONFIGURE_PARAMS_VER;

            // 現在のパラメータをコピー
            reconfig_params.reInitEncodeParams = self.init_params;
            // encodeConfig ポインタは内部の encode_config のコピーを指す必要がある
            let mut new_config = self.encode_config;
            reconfig_params.reInitEncodeParams.encodeConfig = &mut new_config;

            // 変更パラメータを上書き
            if let Some(width) = params.width {
                reconfig_params.reInitEncodeParams.encodeWidth = width;
                reconfig_params.reInitEncodeParams.darWidth = width;
            }
            if let Some(height) = params.height {
                reconfig_params.reInitEncodeParams.encodeHeight = height;
                reconfig_params.reInitEncodeParams.darHeight = height;
            }
            if let Some(fps_num) = params.framerate_num {
                reconfig_params.reInitEncodeParams.frameRateNum = fps_num;
            }
            if let Some(fps_den) = params.framerate_den {
                reconfig_params.reInitEncodeParams.frameRateDen = fps_den;
            }
            if let Some(bitrate) = params.average_bitrate {
                new_config.rcParams.averageBitRate = bitrate;
            }
            if let Some(max_br) = params.max_bitrate {
                new_config.rcParams.maxBitRate = max_br;
            }

            let status = self
                .encoder
                .nvEncReconfigureEncoder
                .map(|f| f(self.h_encoder, &mut reconfig_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncReconfigureEncoder")?;

            // 成功したので保存パラメータを更新
            self.encode_config = new_config;
            self.init_params = reconfig_params.reInitEncodeParams;
            self.init_params.encodeConfig = &mut self.encode_config;

            if let Some(width) = params.width {
                self.width = width;
            }
            if let Some(height) = params.height {
                self.height = height;
            }
            if params.width.is_some() || params.height.is_some() {
                self.expected_frame_size =
                    self.buffer_format_enum.frame_size(self.width, self.height);
            }
            if let Some(fps_den) = params.framerate_den {
                self.framerate_den = fps_den as u64;
            }

            Ok(())
        }
    }

    fn initialize_encoder(&mut self, config: &EncoderConfig) -> Result<(), Error> {
        unsafe {
            // コーデック固有の設定を取得
            let (codec_guid, profile_guid, idr_period) = match &config.codec {
                CodecConfig::H264(h264) => {
                    let profile = h264.profile.unwrap_or(H264Profile::Main).to_sys();
                    let idr = h264.idr_period.unwrap_or_else(|| {
                        config.gop_length.unwrap_or(sys::NVENC_INFINITE_GOPLENGTH)
                    });
                    (sys::NV_ENC_CODEC_H264_GUID, profile, idr)
                }
                CodecConfig::Hevc(hevc) => {
                    let profile = hevc.profile.unwrap_or(HevcProfile::Main).to_sys();
                    let idr = hevc.idr_period.unwrap_or_else(|| {
                        config.gop_length.unwrap_or(sys::NVENC_INFINITE_GOPLENGTH)
                    });
                    (sys::NV_ENC_CODEC_HEVC_GUID, profile, idr)
                }
                CodecConfig::Av1(av1) => {
                    let profile = av1.profile.unwrap_or(Av1Profile::Main).to_sys();
                    let idr = av1.idr_period.unwrap_or_else(|| {
                        config.gop_length.unwrap_or(sys::NVENC_INFINITE_GOPLENGTH)
                    });
                    (sys::NV_ENC_CODEC_AV1_GUID, profile, idr)
                }
            };

            // プリセット設定を取得
            let mut preset_config: sys::NV_ENC_PRESET_CONFIG = std::mem::zeroed();
            preset_config.version = sys::NV_ENC_PRESET_CONFIG_VER;
            preset_config.presetCfg.version = sys::NV_ENC_CONFIG_VER;

            let status = self
                .encoder
                .nvEncGetEncodePresetConfigEx
                .map(|f| {
                    f(
                        self.h_encoder,
                        codec_guid,
                        config.preset.to_sys(),
                        config.tuning_info.to_sys(),
                        &mut preset_config,
                    )
                })
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncGetEncodePresetConfigEx")?;

            // エンコーダーパラメータを初期化
            let mut init_params: sys::NV_ENC_INITIALIZE_PARAMS = std::mem::zeroed();
            let mut encode_config: sys::NV_ENC_CONFIG = preset_config.presetCfg;

            init_params.version = sys::NV_ENC_INITIALIZE_PARAMS_VER;
            init_params.encodeGUID = codec_guid;
            init_params.presetGUID = config.preset.to_sys();
            init_params.encodeWidth = config.width;
            init_params.encodeHeight = config.height;
            init_params.darWidth = config.width;
            init_params.darHeight = config.height;
            init_params.frameRateNum = config.framerate_num;
            init_params.frameRateDen = config.framerate_den;
            init_params.enablePTD = 1;

            init_params.encodeConfig = &mut encode_config;
            init_params.maxEncodeWidth = config.max_encode_width.unwrap_or(config.width);
            init_params.maxEncodeHeight = config.max_encode_height.unwrap_or(config.height);
            init_params.tuningInfo = config.tuning_info.to_sys();

            {
                encode_config.version = sys::NV_ENC_CONFIG_VER;
                encode_config.profileGUID = profile_guid;
                encode_config.gopLength =
                    config.gop_length.unwrap_or(sys::NVENC_INFINITE_GOPLENGTH);
                encode_config.frameIntervalP = config.frame_interval_p as i32;
                encode_config.rcParams.rateControlMode = config.rate_control_mode.to_sys();

                // ビットレート設定
                if config.rate_control_mode != RateControlMode::ConstQp {
                    let bitrate = config.average_bitrate.ok_or_else(|| {
                        Error::new_custom(
                            "initialize_encoder",
                            "average_bitrate must be specified when not using ConstQp mode",
                        )
                    })?;
                    encode_config.rcParams.averageBitRate = bitrate;
                    encode_config.rcParams.maxBitRate = bitrate;
                }

                // コーデック固有の設定
                match &config.codec {
                    CodecConfig::H264(_) => {
                        encode_config.encodeCodecConfig.h264Config.idrPeriod = idr_period;
                    }
                    CodecConfig::Hevc(_) => {
                        encode_config.encodeCodecConfig.hevcConfig.idrPeriod = idr_period;
                    }
                    CodecConfig::Av1(_) => {
                        encode_config.encodeCodecConfig.av1Config.idrPeriod = idr_period;
                    }
                }
            }

            // エンコーダーを初期化
            let status = self
                .encoder
                .nvEncInitializeEncoder
                .map(|f| f(self.h_encoder, &mut init_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncInitializeEncoder")?;

            // 再構成で使うために初期化パラメータを保存
            self.encode_config = encode_config;
            self.init_params = init_params;
            // init_params.encodeConfig はローカルの encode_config を指していたので、
            // self.encode_config を指すように修正する
            self.init_params.encodeConfig = &mut self.encode_config;

            Ok(())
        }
    }

    /// シーケンスパラメータ（SPS/PPS または Sequence Header OBU）を取得する
    ///
    /// H.264/HEVC の場合は SPS/PPS、AV1 の場合は Sequence Header OBU を取得します。
    pub fn get_sequence_params(&mut self) -> Result<Vec<u8>, Error> {
        self.lib
            .with_context(self.ctx, || self.get_sequence_params_inner())
    }

    fn get_sequence_params_inner(&self) -> Result<Vec<u8>, Error> {
        unsafe {
            // シーケンスパラメータを格納するバッファを確保
            let mut payload_buffer = vec![0u8; sys::NV_MAX_SEQ_HDR_LEN as usize];
            let mut out_size: u32 = 0; // 実際のサイズを受け取る変数

            let mut seq_params: sys::NV_ENC_SEQUENCE_PARAM_PAYLOAD = std::mem::zeroed();
            seq_params.version = sys::NV_ENC_SEQUENCE_PARAM_PAYLOAD_VER;
            seq_params.spsppsBuffer = payload_buffer.as_mut_ptr() as *mut std::ffi::c_void;
            seq_params.inBufferSize = sys::NV_MAX_SEQ_HDR_LEN;
            seq_params.outSPSPPSPayloadSize = &mut out_size;

            let status = self
                .encoder
                .nvEncGetSequenceParams
                .map(|f| f(self.h_encoder, &mut seq_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);

            Error::check_nvenc(status, "nvEncGetSequenceParams")?;

            // 実際に書き込まれたサイズに合わせてバッファをリサイズ
            payload_buffer.truncate(out_size as usize);

            Ok(payload_buffer)
        }
    }

    /// フレームデータをエンコードする
    pub fn encode(&mut self, frame_data: &[u8], options: &EncodeOptions) -> Result<(), Error> {
        let expected_size = self.expected_frame_size;

        if frame_data.len() != expected_size {
            return Err(Error::new_custom("encode", "invalid frame data size"));
        }

        self.lib
            .clone()
            .with_context(self.ctx, || self.encode_inner(frame_data, options))
    }

    fn encode_inner(&mut self, frame_data: &[u8], options: &EncodeOptions) -> Result<(), Error> {
        // 入力データをデバイスにコピー
        let (device_input, _device_guard) = self.copy_input_data_to_device(frame_data)?;

        // CUDA デバイスメモリを入力リソースとして登録
        let (registered_resource, _registered_guard) =
            self.register_input_resource(device_input)?;

        // 登録したリソースをマップ
        let (mapped_resource, _mapped_guard) = self.map_input_resource(registered_resource)?;

        // 出力ビットストリームバッファを割り当て
        let (output_buffer, _bitstream_guard) = self.create_output_bitstream_buffer()?;

        // ピクチャをエンコード
        self.encode_picture(mapped_resource, output_buffer, options)?;

        // ビットストリームをロックしてエンコード済みデータをコピー
        let encoded_frame = self.lock_and_copy_bitstream(output_buffer)?;

        // エンコード済みフレームを保存
        self.encoded_frames.push_back(encoded_frame);

        Ok(())
    }

    fn copy_input_data_to_device(
        &mut self,
        frame_data: &[u8],
    ) -> Result<(sys::CUdeviceptr, ReleaseGuard<impl FnOnce() + use<>>), Error> {
        let mut device_input: sys::CUdeviceptr = 0;
        self.lib.cu_mem_alloc(&mut device_input, frame_data.len())?;

        let lib = self.lib.clone();
        let device_guard = ReleaseGuard::new(move || {
            let _ = lib.cu_mem_free(device_input);
        });

        self.lib
            .cu_memcpy_h_to_d(device_input, frame_data.as_ptr().cast(), frame_data.len())?;

        Ok((device_input, device_guard))
    }

    fn register_input_resource(
        &mut self,
        device_input: sys::CUdeviceptr,
    ) -> Result<
        (
            sys::NV_ENC_REGISTERED_PTR,
            ReleaseGuard<impl FnOnce() + use<>>,
        ),
        Error,
    > {
        unsafe {
            let mut register_resource: sys::NV_ENC_REGISTER_RESOURCE = std::mem::zeroed();
            register_resource.version = sys::NV_ENC_REGISTER_RESOURCE_VER;
            register_resource.resourceType =
                sys::_NV_ENC_INPUT_RESOURCE_TYPE_NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR;
            register_resource.resourceToRegister = device_input as *mut c_void;
            register_resource.width = self.width;
            register_resource.height = self.height;
            register_resource.pitch = self.buffer_format_enum.bytes_per_row(self.width);
            register_resource.bufferFormat = self.buffer_format;
            register_resource.bufferUsage = sys::_NV_ENC_BUFFER_USAGE_NV_ENC_INPUT_IMAGE;

            let status = self
                .encoder
                .nvEncRegisterResource
                .map(|f| f(self.h_encoder, &mut register_resource))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncRegisterResource")?;

            let registered_resource = register_resource.registeredResource;

            let unregister = self.encoder.nvEncUnregisterResource;
            let h_encoder = self.h_encoder;
            let registered_guard = ReleaseGuard::new(move || {
                unregister.map(|f| f(h_encoder, registered_resource));
            });

            Ok((registered_resource, registered_guard))
        }
    }

    fn map_input_resource(
        &mut self,
        registered_resource: sys::NV_ENC_REGISTERED_PTR,
    ) -> Result<(sys::NV_ENC_INPUT_PTR, ReleaseGuard<impl FnOnce() + use<>>), Error> {
        unsafe {
            let mut map_input_resource: sys::NV_ENC_MAP_INPUT_RESOURCE = std::mem::zeroed();
            map_input_resource.version = sys::NV_ENC_MAP_INPUT_RESOURCE_VER;
            map_input_resource.registeredResource = registered_resource;

            let status = self
                .encoder
                .nvEncMapInputResource
                .map(|f| f(self.h_encoder, &mut map_input_resource))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncMapInputResource")?;

            let mapped_resource = map_input_resource.mappedResource;

            let unmap = self.encoder.nvEncUnmapInputResource;
            let h_encoder = self.h_encoder;
            let mapped_guard = ReleaseGuard::new(move || {
                unmap.map(|f| f(h_encoder, mapped_resource));
            });

            Ok((mapped_resource, mapped_guard))
        }
    }

    fn create_output_bitstream_buffer(
        &mut self,
    ) -> Result<(sys::NV_ENC_OUTPUT_PTR, ReleaseGuard<impl FnOnce() + use<>>), Error> {
        unsafe {
            let mut create_bitstream: sys::NV_ENC_CREATE_BITSTREAM_BUFFER = std::mem::zeroed();
            create_bitstream.version = sys::NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

            let status = self
                .encoder
                .nvEncCreateBitstreamBuffer
                .map(|f| f(self.h_encoder, &mut create_bitstream))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncCreateBitstreamBuffer")?;

            let output_buffer = create_bitstream.bitstreamBuffer;

            let destroy = self.encoder.nvEncDestroyBitstreamBuffer;
            let h_encoder = self.h_encoder;
            let bitstream_guard = ReleaseGuard::new(move || {
                destroy.map(|f| f(h_encoder, output_buffer));
            });

            Ok((output_buffer, bitstream_guard))
        }
    }

    fn encode_picture(
        &mut self,
        mapped_resource: sys::NV_ENC_INPUT_PTR,
        output_buffer: sys::NV_ENC_OUTPUT_PTR,
        options: &EncodeOptions,
    ) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.inputWidth = self.width;
            pic_params.inputHeight = self.height;
            pic_params.inputPitch = self.buffer_format_enum.bytes_per_row(self.width);
            pic_params.inputBuffer = mapped_resource;
            pic_params.outputBitstream = output_buffer;
            pic_params.bufferFmt = self.buffer_format;
            pic_params.pictureStruct = sys::_NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME;
            pic_params.inputTimeStamp = self.frame_count * self.framerate_den;
            pic_params.encodePicFlags = options.to_pic_flags();

            self.frame_count += 1;

            let status = self
                .encoder
                .nvEncEncodePicture
                .map(|f| f(self.h_encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncEncodePicture")?;

            Ok(())
        }
    }

    fn lock_and_copy_bitstream(
        &mut self,
        output_buffer: sys::NV_ENC_OUTPUT_PTR,
    ) -> Result<EncodedFrame, Error> {
        unsafe {
            let mut lock_bitstream: sys::NV_ENC_LOCK_BITSTREAM = std::mem::zeroed();
            lock_bitstream.version = sys::NV_ENC_LOCK_BITSTREAM_VER;
            lock_bitstream.outputBitstream = output_buffer;

            let status = self
                .encoder
                .nvEncLockBitstream
                .map(|f| f(self.h_encoder, &mut lock_bitstream))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncLockBitstream")?;

            // どの分岐でも必ず unlock するためのガード
            let unlock_fn = self.encoder.nvEncUnlockBitstream;
            let h_encoder = self.h_encoder;
            let output_bitstream = lock_bitstream.outputBitstream;
            let _unlock_guard = crate::ReleaseGuard::new(move || {
                if let Some(f) = unlock_fn {
                    let _ = f(h_encoder, output_bitstream);
                }
            });

            // ビットストリームがロックされている間にエンコード済みデータをコピー
            let ptr = lock_bitstream.bitstreamBufferPtr as *const u8;
            let size = lock_bitstream.bitstreamSizeInBytes as usize;
            let encoded_data = if ptr.is_null() {
                return Err(Error::new_custom(
                    "nvEncLockBitstream",
                    "bitstreamBufferPtr is null",
                ));
            } else if size == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(ptr, size).to_vec()
            };

            let timestamp = lock_bitstream.outputTimeStamp;
            let picture_type = PictureType::new(lock_bitstream.pictureType);

            Ok(EncodedFrame {
                data: encoded_data,
                timestamp,
                picture_type,
            })
        }
    }

    /// エンコーダーを終了し、残りのフレームを取得する
    pub fn finish(&mut self) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.encodePicFlags = sys::NV_ENC_PIC_FLAG_EOS;
            pic_params.inputTimeStamp = self.frame_count;

            let status = self
                .encoder
                .nvEncEncodePicture
                .map(|f| f(self.h_encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncEncodePicture")?;

            Ok(())
        }
    }

    /// 次のエンコード済みフレームを取得する
    pub fn next_frame(&mut self) -> Option<EncodedFrame> {
        self.encoded_frames.pop_front()
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.lib.with_context(self.ctx, || {
                if let Some(destroy_fn) = self.encoder.nvEncDestroyEncoder {
                    destroy_fn(self.h_encoder);
                }
                Ok(())
            });

            let _ = self.lib.cu_ctx_destroy(self.ctx);
        }
    }
}

impl std::fmt::Debug for Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder")
            .field("ctx", &format_args!("{:p}", self.ctx))
            .field("h_encoder", &format_args!("{:p}", self.h_encoder))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("buffer_format", &self.buffer_format)
            .field("frame_count", &self.frame_count)
            .finish()
    }
}

unsafe impl Send for Encoder {}

/// ピクチャータイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureType {
    /// P フレーム
    P,
    /// B フレーム
    B,
    /// I フレーム
    I,
    /// IDR フレーム
    Idr,
    /// BI フレーム
    Bi,
    /// スキップされたフレーム
    Skipped,
    /// イントラリフレッシュフレーム
    IntraRefresh,
    /// 非参照 P フレーム
    NonRefP,
    /// スイッチフレーム
    Switch,
    /// 不明なフレームタイプ
    Unknown,
}

impl PictureType {
    fn new(pic_type: sys::NV_ENC_PIC_TYPE) -> Self {
        match pic_type {
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_P => PictureType::P,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_B => PictureType::B,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_I => PictureType::I,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_IDR => PictureType::Idr,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_BI => PictureType::Bi,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_SKIPPED => PictureType::Skipped,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_INTRA_REFRESH => PictureType::IntraRefresh,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_NONREF_P => PictureType::NonRefP,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_SWITCH => PictureType::Switch,
            _ => PictureType::Unknown,
        }
    }
}

/// エンコード済みフレーム
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    data: Vec<u8>,
    timestamp: u64,
    picture_type: PictureType,
}

impl EncodedFrame {
    /// エンコードされたデータを取得する
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// エンコードされたデータを取得する（所有権を移動）
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// タイムスタンプを取得する
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// ピクチャータイプを取得する
    pub fn picture_type(&self) -> PictureType {
        self.picture_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用のエンコーダー設定を生成する
    fn test_encoder_config(codec: CodecConfig) -> EncoderConfig {
        EncoderConfig {
            codec,
            width: 640,
            height: 480,
            max_encode_width: None,
            max_encode_height: None,
            framerate_num: 30,
            framerate_den: 1,
            average_bitrate: Some(5_000_000),
            preset: Preset::P4,
            tuning_info: TuningInfo::LOW_LATENCY,
            rate_control_mode: RateControlMode::Vbr,
            gop_length: None,
            frame_interval_p: 1,
            buffer_format: BufferFormat::Nv12,
            device_id: 0,
        }
    }

    #[test]
    fn init_h264_encoder() {
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let _encoder = Encoder::new(config).expect("failed to initialize h264 encoder");
        println!("h264 encoder initialized successfully");
    }

    #[test]
    fn init_h265_encoder() {
        let config = test_encoder_config(CodecConfig::Hevc(HevcEncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let _encoder = Encoder::new(config).expect("failed to initialize h265 encoder");
        println!("h265 encoder initialized successfully");
    }

    #[test]
    fn init_av1_encoder() {
        let config = test_encoder_config(CodecConfig::Av1(Av1EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let _encoder = Encoder::new(config).expect("failed to initialize av1 encoder");
        println!("av1 encoder initialized successfully");
    }

    #[test]
    fn test_get_sequence_params_h264() {
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let mut encoder = Encoder::new(config).expect("failed to create h264 encoder");

        // SPS/PPS を取得
        let seq_params = encoder
            .get_sequence_params()
            .expect("failed to get sequence parameters");

        // シーケンスパラメータが空でないことを確認
        assert!(
            !seq_params.is_empty(),
            "Sequence parameters should not be empty"
        );

        println!("H.264 sequence parameters size: {} bytes", seq_params.len());
    }

    #[test]
    fn test_get_sequence_params_h265() {
        let config = test_encoder_config(CodecConfig::Hevc(HevcEncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let mut encoder = Encoder::new(config).expect("failed to create h265 encoder");

        // VPS/SPS/PPS を取得
        let seq_params = encoder
            .get_sequence_params()
            .expect("failed to get sequence parameters");

        // シーケンスパラメータが空でないことを確認
        assert!(
            !seq_params.is_empty(),
            "Sequence parameters should not be empty"
        );

        println!("H.265 sequence parameters size: {} bytes", seq_params.len());
    }

    #[test]
    fn test_get_sequence_params_av1() {
        let config = test_encoder_config(CodecConfig::Av1(Av1EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let mut encoder = Encoder::new(config).expect("failed to create av1 encoder");

        // Sequence Header OBU を取得
        let seq_params = encoder
            .get_sequence_params()
            .expect("failed to get sequence parameters");

        // シーケンスパラメータが空でないことを確認
        assert!(
            !seq_params.is_empty(),
            "Sequence parameters should not be empty"
        );

        println!("AV1 sequence header size: {} bytes", seq_params.len());
    }

    #[test]
    fn test_encode_h264_black_frame() {
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let mut encoder = Encoder::new(config).expect("failed to create h264 encoder");

        // NV12 形式の黒フレームを準備
        // Y 成分は 16（黒）、UV 成分は 128（ニュートラル）
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;

        let mut frame_data = vec![16u8; y_size + uv_size];
        frame_data[y_size..].fill(128);

        // エンコードを実行
        encoder
            .encode(
                &frame_data,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: false,
                    output_spspps: false,
                },
            )
            .expect("failed to encode black frame");

        // エンコーダーを終了して残りのフレームをフラッシュ
        encoder.finish().expect("failed to finish encoder");

        // エンコード済みフレームを取得
        let mut frames = Vec::new();
        while let Some(frame) = encoder.next_frame() {
            frames.push(frame);
        }

        // 少なくとも 1 フレームはエンコードされるはず
        assert!(!frames.is_empty(), "No encoded frames received");

        // 最初のフレームはキーフレーム（I or IDR）であることを確認
        let first_frame = &frames[0];
        assert!(
            matches!(first_frame.picture_type, PictureType::I | PictureType::Idr),
            "First frame should be a keyframe"
        );
        assert!(
            !first_frame.data.is_empty(),
            "Encoded frame should have data"
        );

        println!(
            "Successfully encoded black frame: {} frames, first frame size: {} bytes",
            frames.len(),
            first_frame.data.len()
        );
    }

    #[test]
    fn test_encode_h265_black_frame() {
        let config = test_encoder_config(CodecConfig::Hevc(HevcEncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let mut encoder = Encoder::new(config).expect("failed to create h265 encoder");

        // NV12 形式の黒フレームを準備
        // Y 成分は 16（黒）、UV 成分は 128（ニュートラル）
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;

        let mut frame_data = vec![16u8; y_size + uv_size];
        frame_data[y_size..].fill(128);

        // エンコードを実行
        encoder
            .encode(
                &frame_data,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: false,
                    output_spspps: false,
                },
            )
            .expect("failed to encode black frame");

        // エンコーダーを終了して残りのフレームをフラッシュ
        encoder.finish().expect("failed to finish encoder");

        // エンコード済みフレームを取得
        let mut frames = Vec::new();
        while let Some(frame) = encoder.next_frame() {
            frames.push(frame);
        }

        // 少なくとも 1 フレームはエンコードされるはず
        assert!(!frames.is_empty(), "No encoded frames received");

        // 最初のフレームはキーフレーム（I or IDR）であることを確認
        let first_frame = &frames[0];
        assert!(
            matches!(first_frame.picture_type, PictureType::I | PictureType::Idr),
            "First frame should be a keyframe"
        );
        assert!(
            !first_frame.data.is_empty(),
            "Encoded frame should have data"
        );

        println!(
            "Successfully encoded black frame: {} frames, first frame size: {} bytes",
            frames.len(),
            first_frame.data.len()
        );
    }

    #[test]
    fn test_encode_av1_black_frame() {
        let config = test_encoder_config(CodecConfig::Av1(Av1EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let mut encoder = Encoder::new(config).expect("failed to create av1 encoder");

        // NV12 形式の黒フレームを準備
        // Y 成分は 16（黒）、UV 成分は 128（ニュートラル）
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;

        let mut frame_data = vec![16u8; y_size + uv_size];
        frame_data[y_size..].fill(128);

        // エンコードを実行
        encoder
            .encode(
                &frame_data,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: false,
                    output_spspps: false,
                },
            )
            .expect("failed to encode black frame");

        // エンコーダーを終了して残りのフレームをフラッシュ
        encoder.finish().expect("failed to finish encoder");

        // エンコード済みフレームを取得
        let mut frames = Vec::new();
        while let Some(frame) = encoder.next_frame() {
            frames.push(frame);
        }

        // 少なくとも 1 フレームはエンコードされるはず
        assert!(!frames.is_empty(), "No encoded frames received");

        // 最初のフレームはキーフレーム（I or IDR）であることを確認
        let first_frame = &frames[0];
        assert!(
            matches!(first_frame.picture_type, PictureType::I | PictureType::Idr),
            "First frame should be a keyframe"
        );
        assert!(
            !first_frame.data.is_empty(),
            "Encoded frame should have data"
        );

        println!(
            "Successfully encoded black frame: {} frames, first frame size: {} bytes",
            frames.len(),
            first_frame.data.len()
        );
    }
}
