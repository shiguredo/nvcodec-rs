use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

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
    fn bytes_per_row(self, width: u32) -> Result<u32, Error> {
        let multiplier = match self {
            // Planar 8bit: 1 byte/pixel
            BufferFormat::Nv12 | BufferFormat::Yv12 | BufferFormat::Iyuv | BufferFormat::Yuv444 => {
                1u32
            }
            // Planar 10bit: 2 bytes/pixel
            BufferFormat::Yuv420_10bit | BufferFormat::Yuv444_10bit => 2,
            // Packed 8bit/10bit: 4 bytes/pixel
            BufferFormat::Argb
            | BufferFormat::Abgr
            | BufferFormat::Argb10
            | BufferFormat::Abgr10 => 4,
        };
        width.checked_mul(multiplier).ok_or_else(|| {
            Error::new_custom("bytes_per_row", "width overflow in pitch calculation")
        })
    }

    /// 指定された幅と高さに対するフレームデータのバイトサイズを計算する
    fn frame_size(self, width: u32, height: u32) -> Result<usize, Error> {
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| Error::new_custom("frame_size", "width * height overflow"))?;
        let size = match self {
            // YUV 4:2:0 (8bit): width * height * 3 / 2
            BufferFormat::Nv12 | BufferFormat::Yv12 | BufferFormat::Iyuv => {
                pixels.checked_mul(3).map(|v| v / 2)
            }
            // YUV 4:4:4 (8bit): width * height * 3
            BufferFormat::Yuv444 => pixels.checked_mul(3),
            // YUV 4:2:0 (10bit, 2 bytes/pixel): width * height * 3
            BufferFormat::Yuv420_10bit => pixels.checked_mul(3),
            // YUV 4:4:4 (10bit, 2 bytes/pixel): width * height * 6
            BufferFormat::Yuv444_10bit => pixels.checked_mul(6),
            // Packed (8bit, 4 bytes/pixel): width * height * 4
            BufferFormat::Argb | BufferFormat::Abgr => pixels.checked_mul(4),
            // Packed (10bit, 4 bytes/pixel): width * height * 4
            BufferFormat::Argb10 | BufferFormat::Abgr10 => pixels.checked_mul(4),
        };
        size.ok_or_else(|| Error::new_custom("frame_size", "frame size overflow"))
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

struct EncoderState {
    lib: CudaLibrary,
    ctx: sys::CUcontext,
    encoder_api: sys::NV_ENCODE_API_FUNCTION_LIST,
    encoder: *mut c_void,
    width: u32,
    height: u32,
    buffer_format: BufferFormat,
    framerate_den: u64,
    frame_count: u64,
    init_params: sys::NV_ENC_INITIALIZE_PARAMS,
    encode_config: sys::NV_ENC_CONFIG,

    // バッファプール
    n_encoder_buffer: usize,
    device_inputs: Vec<sys::CUdeviceptr>,
    registered_resources: Vec<sys::NV_ENC_REGISTERED_PTR>,
    bitstream_buffers: Vec<sys::NV_ENC_OUTPUT_PTR>,

    // パイプライン状態
    i_to_send: usize,
    i_got: usize,
    mapped_inputs: Vec<Option<sys::NV_ENC_INPUT_PTR>>,
}

unsafe impl Send for EncoderState {}

impl EncoderState {
    fn new(config: &EncoderConfig) -> Result<Self, Error> {
        unsafe {
            let lib = CudaLibrary::load()?;

            // CUDA context の初期化
            let mut ctx = ptr::null_mut();
            let ctx_flags = 0; // デフォルトのコンテキストフラグ
            lib.cu_ctx_create(&mut ctx, ctx_flags, config.device_id)?;

            let lib_clone = lib.clone();
            let ctx_guard = ReleaseGuard::new(move || {
                let _ = lib_clone.cu_ctx_destroy(ctx);
            });

            // NVENC 操作のために CUDA context をアクティブ化し、エンコードセッションを開く
            let (encoder_api, encoder) = lib.with_context(ctx, || {
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

                let mut encoder = ptr::null_mut();
                let status = encoder_api
                    .nvEncOpenEncodeSessionEx
                    .map(|f| f(&mut open_session_params, &mut encoder))
                    .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
                Error::check_nvenc(status, "nvEncOpenEncodeSessionEx")?;

                Ok((encoder_api, encoder))
            })?;

            let n_encoder_buffer = config.frame_interval_p as usize + 3;

            let mut state = Self {
                lib: lib.clone(),
                ctx,
                encoder_api,
                encoder,
                width: config.width,
                height: config.height,
                buffer_format: config.buffer_format,
                framerate_den: config.framerate_den as u64,
                frame_count: 0,
                init_params: std::mem::zeroed(),
                encode_config: std::mem::zeroed(),
                n_encoder_buffer,
                device_inputs: Vec::with_capacity(n_encoder_buffer),
                registered_resources: Vec::with_capacity(n_encoder_buffer),
                bitstream_buffers: Vec::with_capacity(n_encoder_buffer),
                i_to_send: 0,
                i_got: 0,
                mapped_inputs: vec![None; n_encoder_buffer],
            };

            // デフォルトパラメータでエンコーダーを初期化
            lib.with_context(ctx, || {
                state.initialize_encoder(config)?;
                state.init_buffer_pool()?;
                Ok(())
            })?;

            ctx_guard.cancel();

            Ok(state)
        }
    }

    /// 指定コーデックのエンコーダのケーパビリティをクエリする
    fn query_caps(codec: EncoderCodec, device_id: i32) -> Result<EncoderCaps, Error> {
        let codec_guid = match codec {
            EncoderCodec::H264 => sys::NV_ENC_CODEC_H264_GUID,
            EncoderCodec::Hevc => sys::NV_ENC_CODEC_HEVC_GUID,
            EncoderCodec::Av1 => sys::NV_ENC_CODEC_AV1_GUID,
        };
        Self::query_caps_with_codec(device_id, codec_guid)
    }

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

                let mut encoder = ptr::null_mut();
                let status = encoder_api
                    .nvEncOpenEncodeSessionEx
                    .map(|f| f(&mut open_session_params, &mut encoder))
                    .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
                Error::check_nvenc(status, "nvEncOpenEncodeSessionEx")?;

                // セッションを確実に閉じるためのガード
                let destroy_fn = encoder_api.nvEncDestroyEncoder;
                let _ = ReleaseGuard::new(move || {
                    if let Some(f) = destroy_fn {
                        f(encoder);
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
                        .map(|f| f(encoder, codec_guid, &mut caps_param, &mut caps_val))
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
    fn reconfigure(&mut self, params: ReconfigureParams) -> Result<(), Error> {
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
                .encoder_api
                .nvEncReconfigureEncoder
                .map(|f| f(self.encoder, &mut reconfig_params))
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
                self.cleanup_buffer_pool();
                self.init_buffer_pool()?;
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
                .encoder_api
                .nvEncGetEncodePresetConfigEx
                .map(|f| {
                    f(
                        self.encoder,
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

            init_params.encodeConfig = &mut encode_config;

            let status = self
                .encoder_api
                .nvEncInitializeEncoder
                .map(|f| f(self.encoder, &mut init_params))
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
    fn get_sequence_params(&self) -> Result<Vec<u8>, Error> {
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
                .encoder_api
                .nvEncGetSequenceParams
                .map(|f| f(self.encoder, &mut seq_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);

            Error::check_nvenc(status, "nvEncGetSequenceParams")?;

            // 実際に書き込まれたサイズに合わせてバッファをリサイズ
            payload_buffer.truncate(out_size as usize);

            Ok(payload_buffer)
        }
    }

    /// バッファプールを初期化する
    fn init_buffer_pool(&mut self) -> Result<(), Error> {
        let frame_size = self.buffer_format.frame_size(self.width, self.height)?;

        for i in 0..self.n_encoder_buffer {
            // デバイスメモリの確保
            let mut device_ptr: sys::CUdeviceptr = 0;
            self.lib.cu_mem_alloc(&mut device_ptr, frame_size)?;
            self.device_inputs.push(device_ptr);

            // リソース登録
            let mut register_resource: sys::NV_ENC_REGISTER_RESOURCE =
                unsafe { std::mem::zeroed() };
            register_resource.version = sys::NV_ENC_REGISTER_RESOURCE_VER;
            register_resource.resourceType =
                sys::_NV_ENC_INPUT_RESOURCE_TYPE_NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR;
            register_resource.resourceToRegister = device_ptr as *mut c_void;
            register_resource.width = self.width;
            register_resource.height = self.height;
            register_resource.pitch = self.buffer_format.bytes_per_row(self.width)?;
            register_resource.bufferFormat = self.buffer_format.to_sys();
            register_resource.bufferUsage = sys::_NV_ENC_BUFFER_USAGE_NV_ENC_INPUT_IMAGE;

            let status = self
                .encoder_api
                .nvEncRegisterResource
                .map(|f| unsafe { f(self.encoder, &mut register_resource) })
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncRegisterResource")?;

            self.registered_resources
                .push(register_resource.registeredResource);

            // ビットストリームバッファの作成
            let mut create_bs: sys::NV_ENC_CREATE_BITSTREAM_BUFFER = unsafe { std::mem::zeroed() };
            create_bs.version = sys::NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

            let status = self
                .encoder_api
                .nvEncCreateBitstreamBuffer
                .map(|f| unsafe { f(self.encoder, &mut create_bs) })
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncCreateBitstreamBuffer")?;

            self.bitstream_buffers.push(create_bs.bitstreamBuffer);

            // mapped_inputs は事前に vec![None; n_encoder_buffer] で初期化済み
            debug_assert!(self.mapped_inputs[i].is_none());
        }

        Ok(())
    }

    fn cleanup_buffer_pool(&mut self) {
        if self.device_inputs.is_empty() {
            return; // 既にクリーンアップ済み
        }
        let _ = self.lib.with_context(self.ctx, || {
            for i in 0..self.n_encoder_buffer {
                if let Some(mapped) = self.mapped_inputs[i].take() {
                    let _ = self
                        .encoder_api
                        .nvEncUnmapInputResource
                        .map(|f| unsafe { f(self.encoder, mapped) });
                }
                let _ = self
                    .encoder_api
                    .nvEncUnregisterResource
                    .map(|f| unsafe { f(self.encoder, self.registered_resources[i]) });
                let _ = self
                    .encoder_api
                    .nvEncDestroyBitstreamBuffer
                    .map(|f| unsafe { f(self.encoder, self.bitstream_buffers[i]) });
                let _ = self.lib.cu_mem_free(self.device_inputs[i]);
            }
            Ok(())
        });
        self.device_inputs.clear();
        self.registered_resources.clear();
        self.bitstream_buffers.clear();
        self.mapped_inputs.fill(None);
    }

    fn map_resource(&mut self, bfr_idx: usize) -> Result<sys::NV_ENC_INPUT_PTR, Error> {
        unsafe {
            let mut map_input_resource: sys::NV_ENC_MAP_INPUT_RESOURCE = std::mem::zeroed();
            map_input_resource.version = sys::NV_ENC_MAP_INPUT_RESOURCE_VER;
            map_input_resource.registeredResource = self.registered_resources[bfr_idx];

            let status = self
                .encoder_api
                .nvEncMapInputResource
                .map(|f| f(self.encoder, &mut map_input_resource))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncMapInputResource")?;

            self.mapped_inputs[bfr_idx] = Some(map_input_resource.mappedResource);
            Ok(map_input_resource.mappedResource)
        }
    }

    fn unmap_resource(&mut self, bfr_idx: usize) {
        let lib = self.lib.clone();
        let _ = lib.with_context(self.ctx, || {
            self.unmap_resource_inner(bfr_idx);
            Ok(())
        });
    }

    fn unmap_resource_inner(&mut self, bfr_idx: usize) {
        Self::unmap_resource_inner_static(
            &mut self.mapped_inputs,
            bfr_idx,
            &self.encoder_api,
            self.encoder,
        );
    }

    fn unmap_resource_inner_static(
        mapped_inputs: &mut [Option<sys::NV_ENC_INPUT_PTR>],
        bfr_idx: usize,
        encoder_api: &sys::NV_ENCODE_API_FUNCTION_LIST,
        encoder: *mut c_void,
    ) {
        unsafe {
            let Some(mapped) = mapped_inputs[bfr_idx].take() else {
                return;
            };
            let _ = encoder_api
                .nvEncUnmapInputResource
                .map(|f| f(encoder, mapped));
        }
    }

    fn encode_frame(
        &mut self,
        bfr_idx: usize,
        data: &[u8],
        options: &EncodeOptions,
    ) -> Result<(), Error> {
        let lib = self.lib.clone();
        lib.with_context(self.ctx, || self.encode_frame_inner(bfr_idx, data, options))
    }

    fn encode_frame_inner(
        &mut self,
        bfr_idx: usize,
        data: &[u8],
        options: &EncodeOptions,
    ) -> Result<(), Error> {
        unsafe {
            let expected_size = self.buffer_format.frame_size(self.width, self.height)?;
            if data.len() != expected_size {
                return Err(Error::new_custom("encode", "invalid frame data size"));
            }

            // デバイスメモリにコピー
            self.lib.cu_memcpy_h_to_d(
                self.device_inputs[bfr_idx],
                data.as_ptr().cast(),
                data.len(),
            )?;

            // リソースマップ
            let mapped = self.map_resource(bfr_idx)?;

            // エラー時に自動で unmap するガード
            let mapped_inputs = &mut self.mapped_inputs;
            let encoder = self.encoder;
            let encoder_api = &self.encoder_api;
            let unmap_guard = ReleaseGuard::new(|| {
                Self::unmap_resource_inner_static(mapped_inputs, bfr_idx, encoder_api, encoder);
            });

            // エンコード
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.inputWidth = self.width;
            pic_params.inputHeight = self.height;
            pic_params.inputPitch = self.buffer_format.bytes_per_row(self.width)?;
            pic_params.inputBuffer = mapped;
            pic_params.outputBitstream = self.bitstream_buffers[bfr_idx];
            pic_params.bufferFmt = self.buffer_format.to_sys();
            pic_params.pictureStruct = sys::_NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME;
            pic_params.inputTimeStamp = self.frame_count * self.framerate_den;
            pic_params.encodePicFlags = options.to_pic_flags();
            pic_params.frameIdx = self.i_to_send as u32;

            self.frame_count += 1;

            let status = self
                .encoder_api
                .nvEncEncodePicture
                .map(|f| f(self.encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);

            Error::check_nvenc(status, "nvEncEncodePicture")?;

            // エンコード成功時はリソースを mapped 状態に保つ
            // （後続の drain で unmap_resource が担当する）
            unmap_guard.cancel();

            Ok(())
        }
    }
}

impl EncoderState {
    /// エンコーダーを終了し、残りのフレームを取得する
    fn send_eos(&mut self) -> Result<(), Error> {
        let lib = self.lib.clone();
        lib.with_context(self.ctx, || self.send_eos_inner())
    }

    fn send_eos_inner(&mut self) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.encodePicFlags = sys::NV_ENC_PIC_FLAG_EOS;
            pic_params.inputTimeStamp = self.frame_count;

            let status = self
                .encoder_api
                .nvEncEncodePicture
                .map(|f| f(self.encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check_nvenc(status, "nvEncEncodePicture")?;

            Ok(())
        }
    }
}

impl Drop for EncoderState {
    fn drop(&mut self) {
        unsafe {
            self.cleanup_buffer_pool();

            let _ = self.lib.with_context(self.ctx, || {
                if let Some(destroy_fn) = self.encoder_api.nvEncDestroyEncoder {
                    destroy_fn(self.encoder);
                }
                Ok(())
            });

            let _ = self.lib.cu_ctx_destroy(self.ctx);
        }
    }
}

/// エンコーダー
///
/// 内部で専用のワーカースレッドを起動し、非同期でエンコードを行う。
/// エンコードが完了すると、コンストラクタで渡したコールバックがワーカースレッド上で即座に呼び出される。
pub struct Encoder<T> {
    job_tx: Sender<Job<T>>,
    worker: Option<JoinHandle<()>>,
    drain_handle: Option<JoinHandle<()>>,
}

/// drain スレッドへのリクエスト
///
/// worker スレッドから drain スレッドへ mpsc 経由で送信される。
///
/// `nvEncLockBitstream` を drain スレッドでブロッキング実行するために必要な
/// コンテキストをすべて含んでいる。
struct DrainRequest {
    lib: CudaLibrary,
    ctx: sys::CUcontext,
    lock_fn: sys::PNVENCLOCKBITSTREAM,
    unlock_fn: sys::PNVENCUNLOCKBITSTREAM,
    encoder: *mut c_void,
    /// ロック対象のビットストリームバッファ
    output_bitstream: sys::NV_ENC_OUTPUT_PTR,
}

unsafe impl Send for DrainRequest {}

/// worker スレッドが受信するメッセージ
///
/// 外部 API（encode/flush/reconfigure）からのジョブと、
/// drain スレッドからの完了通知（DrainResult）の両方が
/// この単一チャネルに集約される。
enum Job<T> {
    Encode {
        data: Vec<u8>,
        options: EncodeOptions,
        user_data: T,
    },
    Reconfigure {
        params: ReconfigureParams,
        done: SyncSender<Result<(), Error>>,
    },
    GetSequenceParams {
        done: SyncSender<Result<Vec<u8>, Error>>,
    },
    Flush {
        done: SyncSender<()>,
    },
    Terminate,
    DrainResult {
        result: Result<(Vec<u8>, u64, PictureType), Error>,
    },
}

impl<T: Send + 'static> Encoder<T> {
    /// エンコーダーを生成する。
    ///
    /// 2 つの内部スレッドが起動される:
    /// - worker スレッド（`nvcodec-encoder`）: job の受信、フレーム送信、バッファ管理
    /// - drain スレッド（`nvcodec-drain`）: NVENC のエンコード待機とエンコード済みデータの取り出し
    pub fn new<F>(config: EncoderConfig, mut callback: F) -> Result<Self, Error>
    where
        F: FnMut(Result<EncodedFrame<T>, Error>) + Send + 'static,
    {
        let (job_tx, job_rx) = mpsc::channel::<Job<T>>();
        let (drain_tx, drain_rx) = mpsc::channel::<DrainRequest>();

        let state = EncoderState::new(&config)?;

        // drain スレッドを起動
        let drain_job_tx = job_tx.clone();
        let drain_handle = std::thread::Builder::new()
            .name("nvcodec-drain".into())
            .spawn(move || {
                drain_thread_loop::<T>(drain_rx, drain_job_tx);
            })
            .map_err(|_e| Error::new_custom("Encoder::new", "failed to spawn drain thread"))?;

        // ワーカースレッドを起動
        let worker = std::thread::Builder::new()
            .name("nvcodec-encoder".into())
            .spawn(move || {
                run_worker(state, &mut callback, job_rx, drain_tx);
            })
            .map_err(|_e| Error::new_custom("Encoder::new", "failed to spawn encoder thread"))?;

        Ok(Self {
            job_tx,
            worker: Some(worker),
            drain_handle: Some(drain_handle),
        })
    }

    /// フレームをエンコードする
    ///
    /// フレームデータとオプションをワーカースレッドに送信し、即座に戻る。
    /// エンコードが完了すると、コンストラクタで渡したコールバックが呼び出される。
    pub fn encode(&self, data: &[u8], options: &EncodeOptions, user_data: T) -> Result<(), Error> {
        self.job_tx
            .send(Job::Encode {
                data: data.to_vec(),
                options: options.clone(),
                user_data,
            })
            .map_err(|_| Error::new_custom("encode", "encoder worker thread has terminated"))
    }

    /// 送信済みの未完了フレームがすべて完了するまで待機する
    ///
    /// すべての pending フレームのコールバックが呼び出された後、このメソッドが戻る。
    /// flush 後も encode を継続できる。
    pub fn flush(&self) -> Result<(), Error> {
        let (tx, rx) = mpsc::sync_channel(0);
        self.job_tx
            .send(Job::Flush { done: tx })
            .map_err(|_| Error::new_custom("flush", "send failed"))?;
        rx.recv()
            .map_err(|_| Error::new_custom("flush", "recv failed"))?;
        Ok(())
    }

    /// エンコーダパラメータを再構成する
    ///
    /// ビットレートやフレームレート、解像度を動的に変更する。
    /// エンコーダの初期化時に設定された値を基準に、指定されたパラメータのみを上書きする。
    ///
    /// 解像度を変更した直後の最初のエンコードフレームには、呼び出し元が
    /// `EncodeOptions { force_idr: true, output_spspps: true, .. }` を指定する必要がある。
    /// これを怠ると新しい解像度の SPS/PPS がビットストリームに出力されず、デコーダーが再生不能になる。
    pub fn reconfigure(&self, params: ReconfigureParams) -> Result<(), Error> {
        let (tx, rx) = mpsc::sync_channel(0);
        self.job_tx
            .send(Job::Reconfigure { params, done: tx })
            .map_err(|_| Error::new_custom("reconfigure", "send failed"))?;
        rx.recv()
            .map_err(|_| Error::new_custom("reconfigure", "recv failed"))?
    }

    /// シーケンスパラメータ（SPS/PPS または Sequence Header OBU）を取得する
    ///
    /// H.264/HEVC の場合は SPS/PPS、AV1 の場合は Sequence Header OBU を取得します。
    pub fn get_sequence_params(&self) -> Result<Vec<u8>, Error> {
        let (tx, rx) = mpsc::sync_channel(0);
        self.job_tx
            .send(Job::GetSequenceParams { done: tx })
            .map_err(|_| Error::new_custom("get_sequence_params", "send failed"))?;
        rx.recv()
            .map_err(|_| Error::new_custom("get_sequence_params", "recv failed"))?
    }
}

impl<T> Drop for Encoder<T> {
    fn drop(&mut self) {
        // worker スレッドに Terminate を送信して終了待機。
        // run_worker 内で全 in-flight フレームの drain が完了した後、
        // drain スレッドが終了する。
        // それによって drain_tx が drop される。
        let _ = self.job_tx.send(Job::Terminate);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        // drain スレッドの終了を待機。
        // drain_tx の drop を検知することで drain スレッドは自動的に終了する。
        if let Some(drain_handle) = self.drain_handle.take() {
            let _ = drain_handle.join();
        }
    }
}

impl<T> std::fmt::Debug for Encoder<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder").finish_non_exhaustive()
    }
}

unsafe impl<T: Send> Send for Encoder<T> {}

/// 指定コーデックのエンコーダのケーパビリティをクエリする
pub fn query_encoder_caps(codec: EncoderCodec, device_id: i32) -> Result<EncoderCaps, Error> {
    EncoderState::query_caps(codec, device_id)
}

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
pub struct EncodedFrame<T> {
    data: Vec<u8>,
    timestamp: u64,
    picture_type: PictureType,
    user_data: T,
}

impl<T> EncodedFrame<T> {
    /// エンコードされたデータを取得する
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// タイムスタンプを取得する
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// ピクチャータイプを取得する
    pub fn picture_type(&self) -> PictureType {
        self.picture_type
    }

    /// ユーザーデータを取得する
    pub fn user_data(&self) -> &T {
        &self.user_data
    }

    /// エンコードされたデータとユーザーデータを取得する（所有権を移動）
    pub fn into_parts(self) -> (Vec<u8>, T) {
        (self.data, self.user_data)
    }
}

/// drain スレッドのメインループ
///
/// worker スレッドから DrainRequest を受信し、
/// `nvEncLockBitstream` をブロッキング実行する。
/// 完了したら DrainResult を mpsc 経由で worker スレッドに送信する。
///
/// `nvEncLockBitstream` がブロッキングするため、この処理だけ drain スレッドで行う。
/// こうすることで、worker スレッドはメインスレッドからのエンコード投入リクエストと
/// drain スレッドからのエンコード完了処理を同時に受け付けられる
fn drain_thread_loop<T: Send + 'static>(drain_rx: Receiver<DrainRequest>, job_tx: Sender<Job<T>>) {
    while let Ok(req) = drain_rx.recv() {
        let result = lock_and_copy_bitstream(
            &req.lib,
            req.ctx,
            req.lock_fn,
            req.unlock_fn,
            req.encoder,
            req.output_bitstream,
        );
        // drain スレッドは結果の成否にかかわらず worker に送信する。
        // エラーハンドリングはここでは行わない。
        let _ = job_tx.send(Job::DrainResult { result });
    }
}

/// NVENC のビットストリームをロックし、エンコード済みデータをコピーする
fn lock_and_copy_bitstream(
    lib: &CudaLibrary,
    ctx: sys::CUcontext,
    lock_fn: sys::PNVENCLOCKBITSTREAM,
    unlock_fn: sys::PNVENCUNLOCKBITSTREAM,
    encoder: *mut c_void,
    output_bitstream: sys::NV_ENC_OUTPUT_PTR,
) -> Result<(Vec<u8>, u64, PictureType), Error> {
    lib.with_context(ctx, || unsafe {
        let mut lock_bitstream: sys::NV_ENC_LOCK_BITSTREAM = std::mem::zeroed();
        lock_bitstream.version = sys::NV_ENC_LOCK_BITSTREAM_VER;
        lock_bitstream.outputBitstream = output_bitstream;

        let status = lock_fn
            .map(|f| f(encoder, &mut lock_bitstream))
            .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
        Error::check_nvenc(status, "nvEncLockBitstream")?;

        // どの分岐でも必ず unlock するためのガード
        let output_bitstream = lock_bitstream.outputBitstream;
        let _unlock_guard = ReleaseGuard::new(move || {
            if let Some(f) = unlock_fn {
                let _ = f(encoder, output_bitstream);
            }
        });

        // ビットストリームがロックされている間にエンコード済みデータをコピー
        let ptr = lock_bitstream.bitstreamBufferPtr as *const u8;
        let size = lock_bitstream.bitstreamSizeInBytes as usize;

        if ptr.is_null() {
            return Err(Error::new_custom(
                "nvEncLockBitstream",
                "bitstreamBufferPtr is null",
            ));
        }

        let data = std::slice::from_raw_parts(ptr, size).to_vec();

        let timestamp = lock_bitstream.outputTimeStamp;
        let picture_type = PictureType::new(lock_bitstream.pictureType);

        Ok((data, timestamp, picture_type))
    })
}

/// worker スレッドのメインループ
///
/// mpsc から Job<T> を受信し、エンコードリクエストの送信と状態管理を行う。
/// nvEncLockBitstream の待機は専用の drain スレッドに委譲される。
///
/// # アーキテクチャ
///
/// ```text
///                                      (DrainRequest の送信)
/// [外部 API] --job_tx--> [worker スレッド] --drain_tx--> [drain スレッド]
///                                         <- job_tx ---
///                                       (DrainResult の返送)
/// ```
///
/// - worker スレッド: フレーム送信（encode_frame）、バッファ管理、callback 呼び出し
/// - drain スレッド: nvEncLockBitstream（ブロッキング）を実行し、結果を job_tx 経由で返送
fn run_worker<F, T>(
    mut state: EncoderState,
    callback: &mut F,
    job_rx: Receiver<Job<T>>,
    drain_tx: Sender<DrainRequest>,
) where
    F: FnMut(Result<EncodedFrame<T>, Error>) + Send + 'static,
    T: Send + 'static,
{
    // user_data を保持するキュー。
    let mut pending_user_data: VecDeque<T> = VecDeque::new();
    // drain リクエストを送信済みのフレーム数。
    // i_got <= i_in_flight <= i_to_send
    let mut i_in_flight = 0;

    while let Ok(job) = job_rx.recv() {
        match job {
            Job::DrainResult { result } => {
                consume_drain_result(&mut state, result, &mut pending_user_data, callback);
            }
            Job::Encode {
                data,
                options,
                user_data,
            } => {
                // バッファが満杯の場合はエラー callback を実行する
                if state.i_to_send - state.i_got >= state.n_encoder_buffer {
                    callback(Err(Error::new_custom("encode", "encoder buffer is full")));
                    continue;
                }

                let bfr_idx = state.i_to_send % state.n_encoder_buffer;
                let encode_result = state.encode_frame(bfr_idx, &data, &options);

                match encode_result {
                    Ok(()) => {
                        // user_data を pending キューに追加。
                        // 後続の DrainResult で pop_front される。
                        pending_user_data.push_back(user_data);
                        state.i_to_send += 1;
                        // 新たに送信したフレームの drain リクエストを送信
                        if !send_pending_drain_requests(
                            &drain_tx,
                            &state,
                            &mut i_in_flight,
                            callback,
                        ) {
                            return;
                        }
                    }
                    Err(e) => {
                        callback(Err(e));
                    }
                }
            }
            Job::Reconfigure { params, done } => {
                // バッファプール再構築との競合を防ぐため、
                // 全 in-flight フレームを drain してから reconfigure を実行する。
                if !send_pending_drain_requests(&drain_tx, &state, &mut i_in_flight, callback) {
                    return;
                }
                if !wait_all_drains(
                    &mut state,
                    &mut pending_user_data,
                    callback,
                    &job_rx,
                    "reconfigure",
                ) {
                    return;
                }
                let _ = done.send(state.reconfigure(params));
            }
            Job::Flush { done } => {
                // 全 in-flight フレームが drain されるまで待機する
                if !send_pending_drain_requests(&drain_tx, &state, &mut i_in_flight, callback) {
                    return;
                }
                if !wait_all_drains(
                    &mut state,
                    &mut pending_user_data,
                    callback,
                    &job_rx,
                    "flush",
                ) {
                    return;
                }
                let _ = done.send(());
            }
            Job::GetSequenceParams { done } => {
                // GetSequenceParams は drain を必要としないので、
                // そのままシーケンスパラメータを返却する。
                let _ = done.send(state.get_sequence_params());
            }
            Job::Terminate => {
                // NVENC に EOS を送信し、エンコーダの内部パイプラインをフラッシュする。
                let _ = state.send_eos();

                // EOS 送信後に残っている全フレームを drain する。
                if !send_pending_drain_requests(&drain_tx, &state, &mut i_in_flight, callback) {
                    return;
                }
                if !wait_all_drains(
                    &mut state,
                    &mut pending_user_data,
                    callback,
                    &job_rx,
                    "terminate",
                ) {
                    return;
                }

                return;
            }
        }
    }
}

/// drain スレッドから受信した結果を消費し、後片付けして callback を呼び出す
fn consume_drain_result<F, T>(
    state: &mut EncoderState,
    result: Result<(Vec<u8>, u64, PictureType), Error>,
    pending_user_data: &mut VecDeque<T>,
    callback: &mut F,
) where
    F: FnMut(Result<EncodedFrame<T>, Error>) + Send + 'static,
    T: Send + 'static,
{
    let bfr_idx = state.i_got % state.n_encoder_buffer;

    // drain が完了したので mapped resource を解放し、
    // 次の encode_frame で再利用可能にする
    state.unmap_resource(bfr_idx);
    state.i_got += 1;

    match result {
        Ok((data, timestamp, picture_type)) => {
            // pending_user_data は送信順に push されているため、
            // pop_front で対応する user_data が取得できる
            if let Some(user_data) = pending_user_data.pop_front() {
                callback(Ok(EncodedFrame {
                    data,
                    timestamp,
                    picture_type,
                    user_data,
                }));
            } else {
                callback(Err(Error::new_custom(
                    "consume_drain_result",
                    "missing user data",
                )));
            }
        }
        Err(e) => {
            // エラー発生時は全 pending データをクリアする。
            pending_user_data.clear();
            callback(Err(e));
        }
    }
}

/// 未送信の drain リクエストをすべて送信する
fn send_pending_drain_requests<F, T>(
    drain_tx: &Sender<DrainRequest>,
    state: &EncoderState,
    i_in_flight: &mut usize,
    callback: &mut F,
) -> bool
where
    F: FnMut(Result<EncodedFrame<T>, Error>) + Send + 'static,
    T: Send + 'static,
{
    while *i_in_flight < state.i_to_send {
        let bfr_idx = *i_in_flight % state.n_encoder_buffer;
        let result = drain_tx.send(DrainRequest {
            lib: state.lib.clone(),
            ctx: state.ctx,
            lock_fn: state.encoder_api.nvEncLockBitstream,
            unlock_fn: state.encoder_api.nvEncUnlockBitstream,
            encoder: state.encoder,
            output_bitstream: state.bitstream_buffers[bfr_idx],
        });
        if result.is_err() {
            callback(Err(Error::new_custom(
                "send_pending_drain_requests",
                "drain thread has terminated",
            )));
            return false;
        };
        *i_in_flight += 1;
    }
    true
}

/// 全 in-flight フレームの drain 完了を待機する
///
/// drain 待機中に DrainResult 以外のメッセージが届いた場合は
/// エラー callback を呼ぶ。
/// job_rx が切断された場合は `false` を返す
/// （呼び出し元の run_worker が return すべきことを示す）。
fn wait_all_drains<F, T>(
    state: &mut EncoderState,
    pending_user_data: &mut VecDeque<T>,
    callback: &mut F,
    job_rx: &Receiver<Job<T>>,
    context: &'static str,
) -> bool
where
    F: FnMut(Result<EncodedFrame<T>, Error>) + Send + 'static,
    T: Send + 'static,
{
    while state.i_got < state.i_to_send {
        match job_rx.recv() {
            Ok(Job::DrainResult { result }) => {
                consume_drain_result(state, result, pending_user_data, callback);
            }
            Ok(_) => {
                callback(Err(Error::new_custom(
                    context,
                    "unexpected message during drain",
                )));
            }
            Err(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

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
        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let _encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to initialize h264 encoder");
    }

    #[test]
    fn init_h265_encoder() {
        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::Hevc(HevcEncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let _encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to initialize h265 encoder");
    }

    #[test]
    fn init_av1_encoder() {
        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::Av1(Av1EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let _encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to initialize av1 encoder");
    }

    #[test]
    fn test_get_sequence_params_h264() {
        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

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
        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::Hevc(HevcEncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h265 encoder");

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
        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::Av1(Av1EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create av1 encoder");

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
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

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
                42,
            )
            .expect("failed to encode black frame");

        // エンコード完了を待機
        encoder.flush().expect("flush failed");

        // エンコード済みフレームを取得
        let frames: Vec<_> = rx.try_iter().collect();
        drop(encoder);

        // 少なくとも 1 フレームはエンコードされるはず
        assert!(!frames.is_empty(), "No encoded frames received");

        // 最初のフレームはキーフレーム（I or IDR）であることを確認
        let first = frames[0].as_ref().expect("First frame should be Ok");
        assert_eq!(first.user_data, 42);
        assert!(
            matches!(first.picture_type(), PictureType::I | PictureType::Idr),
            "First frame should be a keyframe"
        );
        assert!(!first.data().is_empty(), "Encoded frame should have data");
    }

    #[test]
    fn test_encode_h265_black_frame() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(4);
        let config = test_encoder_config(CodecConfig::Hevc(HevcEncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h265 encoder");

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
                7,
            )
            .expect("failed to encode black frame");

        // エンコード完了を待機
        encoder.flush().expect("flush failed");

        // エンコード済みフレームを取得
        let frames: Vec<_> = rx.try_iter().collect();
        drop(encoder);

        // 少なくとも 1 フレームはエンコードされるはず
        assert!(!frames.is_empty(), "No encoded frames received");

        // 最初のフレームはキーフレーム（I or IDR）であることを確認
        let first = frames[0].as_ref().expect("First frame should be Ok");
        assert_eq!(first.user_data, 7);
        assert!(
            matches!(first.picture_type(), PictureType::I | PictureType::Idr),
            "First frame should be a keyframe"
        );
        assert!(!first.data().is_empty(), "Encoded frame should have data");
    }

    #[test]
    fn test_encode_av1_black_frame() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(4);
        let config = test_encoder_config(CodecConfig::Av1(Av1EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create av1 encoder");

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
                3,
            )
            .expect("failed to encode black frame");

        // エンコード完了を待機
        encoder.flush().expect("flush failed");

        // エンコード済みフレームを取得
        let frames: Vec<_> = rx.try_iter().collect();
        drop(encoder);

        // 少なくとも 1 フレームはエンコードされるはず
        assert!(!frames.is_empty(), "No encoded frames received");

        // 最初のフレームはキーフレーム（I or IDR）であることを確認
        let first = frames[0].as_ref().expect("First frame should be Ok");
        assert_eq!(first.user_data, 3);
        assert!(
            matches!(first.picture_type(), PictureType::I | PictureType::Idr),
            "First frame should be a keyframe"
        );
        assert!(!first.data().is_empty(), "Encoded frame should have data");
    }

    #[test]
    fn test_encode_multiple_frames() {
        use std::time::Duration;

        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(8);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // NV12 形式の黒フレームを準備
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;
        let mut frame_data = vec![16u8; y_size + uv_size];
        frame_data[y_size..].fill(128);

        // 5 フレームをエンコード。
        // バッファ満杯を避けるため、30fps 相当のフレーム間隔（33ms）で送信する。
        // これにより drain スレッドが encode() の間に drain を完了できる。
        let frame_interval = Duration::from_millis(33);
        for i in 0..5 {
            encoder
                .encode(
                    &frame_data,
                    &EncodeOptions {
                        force_intra: false,
                        force_idr: false,
                        output_spspps: false,
                    },
                    i,
                )
                .expect("failed to encode frame");
            std::thread::sleep(frame_interval);
        }

        encoder.flush().expect("flush failed");
        drop(encoder);

        // 5 フレームすべてがエンコードされたことを確認
        let frames: Vec<_> = rx.try_iter().collect();
        assert_eq!(frames.len(), 5, "Should have 5 encoded frames");

        for (i, frame) in frames.iter().enumerate() {
            let frame = frame.as_ref().expect("Frame should be Ok");
            assert_eq!(frame.user_data, i as u32);
            assert!(!frame.data().is_empty(), "Frame should have data");
        }
    }

    #[test]
    fn test_flush_without_encode() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // フレームを送信せずに flush してもハングしないことを確認
        encoder.flush().expect("flush failed");
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert!(frames.is_empty(), "No frames expected");
    }

    #[test]
    fn test_reconfigure_h264() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        let width = config.width;
        let height = config.height;

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // フレームレートとビットレートを動的に変更
        encoder
            .reconfigure(ReconfigureParams {
                framerate_num: Some(60),
                framerate_den: Some(1),
                average_bitrate: Some(10_000_000),
                ..Default::default()
            })
            .expect("failed to reconfigure encoder");

        // NV12 形式の黒フレームを準備
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;
        let mut frame_data = vec![16u8; y_size + uv_size];
        frame_data[y_size..].fill(128);

        // 再構成後にエンコードできることを確認
        encoder
            .encode(
                &frame_data,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: false,
                    output_spspps: false,
                },
                1,
            )
            .expect("failed to encode frame after reconfigure");

        encoder.flush().expect("flush failed");
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert!(!frames.is_empty(), "No encoded frames received");

        let first = frames[0].as_ref().expect("First frame should be Ok");
        assert_eq!(first.user_data, 1);
        assert!(!first.data().is_empty(), "Encoded frame should have data");
    }

    /// 解像度変更用のエンコーダー設定を生成する
    /// max_encode_width / max_encode_height を初期解像度より大きく指定する
    fn test_encoder_config_with_max_resolution(
        codec: CodecConfig,
        width: u32,
        height: u32,
        max_width: u32,
        max_height: u32,
    ) -> EncoderConfig {
        EncoderConfig {
            codec,
            width,
            height,
            max_encode_width: Some(max_width),
            max_encode_height: Some(max_height),
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

    /// 指定された解像度の NV12 黒フレームを作成する
    fn create_black_frame(width: u32, height: u32) -> Vec<u8> {
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;
        let mut frame = vec![16u8; y_size + uv_size];
        frame[y_size..].fill(128);
        frame
    }

    #[test]
    fn test_reconfigure_resolution_upscale_h264() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(8);
        let config = test_encoder_config_with_max_resolution(
            CodecConfig::H264(H264EncoderConfig {
                profile: None,
                idr_period: None,
            }),
            640,
            480,
            1280,
            720,
        );

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // 初期解像度でエンコード
        let frame_640x480 = create_black_frame(640, 480);
        encoder
            .encode(
                &frame_640x480,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                1,
            )
            .expect("failed to encode frame at 640x480");

        // 解像度を 1280x720 に拡大
        encoder
            .reconfigure(ReconfigureParams {
                width: Some(1280),
                height: Some(720),
                ..Default::default()
            })
            .expect("failed to reconfigure to 1280x720");

        // 新解像度でエンコード
        let frame_1280x720 = create_black_frame(1280, 720);
        encoder
            .encode(
                &frame_1280x720,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                2,
            )
            .expect("failed to encode frame at 1280x720");

        encoder.flush().expect("flush failed");
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert!(
            frames.len() >= 2,
            "Expected at least 2 encoded frames, got {}",
            frames.len()
        );

        for frame in &frames {
            let frame = frame.as_ref().expect("Frame should be Ok");
            assert!(!frame.data().is_empty(), "Frame should have data");
        }
    }

    #[test]
    fn test_reconfigure_resolution_downscale_h264() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(8);
        let config = test_encoder_config_with_max_resolution(
            CodecConfig::H264(H264EncoderConfig {
                profile: None,
                idr_period: None,
            }),
            1280,
            720,
            1280,
            720,
        );

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // 初期解像度でエンコード
        let frame_1280x720 = create_black_frame(1280, 720);
        encoder
            .encode(
                &frame_1280x720,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                1,
            )
            .expect("failed to encode frame at 1280x720");

        // 解像度を 640x480 に縮小
        encoder
            .reconfigure(ReconfigureParams {
                width: Some(640),
                height: Some(480),
                ..Default::default()
            })
            .expect("failed to reconfigure to 640x480");

        // 新解像度でエンコード
        let frame_640x480 = create_black_frame(640, 480);
        encoder
            .encode(
                &frame_640x480,
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                2,
            )
            .expect("failed to encode frame at 640x480");

        encoder.flush().expect("flush failed");
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert!(
            frames.len() >= 2,
            "Expected at least 2 encoded frames, got {}",
            frames.len()
        );

        for frame in &frames {
            let frame = frame.as_ref().expect("Frame should be Ok");
            assert!(!frame.data().is_empty(), "Frame should have data");
        }
    }

    #[test]
    fn test_reconfigure_width_only_h264() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(8);
        let config = test_encoder_config_with_max_resolution(
            CodecConfig::H264(H264EncoderConfig {
                profile: None,
                idr_period: None,
            }),
            640,
            480,
            960,
            480,
        );

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // 初期解像度でエンコード
        encoder
            .encode(
                &create_black_frame(640, 480),
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                1,
            )
            .expect("failed to encode frame at 640x480");

        // 幅のみ変更
        encoder
            .reconfigure(ReconfigureParams {
                width: Some(960),
                ..Default::default()
            })
            .expect("failed to reconfigure width to 960");

        // 新解像度でエンコード
        encoder
            .encode(
                &create_black_frame(960, 480),
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                2,
            )
            .expect("failed to encode frame at 960x480");

        encoder.flush().expect("flush failed");
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert!(!frames.is_empty(), "No encoded frames received");
        for frame in &frames {
            let frame = frame.as_ref().expect("Frame should be Ok");
            assert!(!frame.data().is_empty(), "Frame should have data");
        }
    }

    #[test]
    fn test_reconfigure_height_only_h264() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(8);
        let config = test_encoder_config_with_max_resolution(
            CodecConfig::H264(H264EncoderConfig {
                profile: None,
                idr_period: None,
            }),
            640,
            480,
            640,
            720,
        );

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // 初期解像度でエンコード
        encoder
            .encode(
                &create_black_frame(640, 480),
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                1,
            )
            .expect("failed to encode frame at 640x480");

        // 高さのみ変更
        encoder
            .reconfigure(ReconfigureParams {
                height: Some(720),
                ..Default::default()
            })
            .expect("failed to reconfigure height to 720");

        // 新解像度でエンコード
        encoder
            .encode(
                &create_black_frame(640, 720),
                &EncodeOptions {
                    force_intra: false,
                    force_idr: true,
                    output_spspps: true,
                },
                2,
            )
            .expect("failed to encode frame at 640x720");

        encoder.flush().expect("flush failed");
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert!(!frames.is_empty(), "No encoded frames received");
        for frame in &frames {
            let frame = frame.as_ref().expect("Frame should be Ok");
            assert!(!frame.data().is_empty(), "Frame should have data");
        }
    }

    #[test]
    fn test_reconfigure_during_encoding_h264() {
        let (tx, rx) = mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(8);
        let config = test_encoder_config_with_max_resolution(
            CodecConfig::H264(H264EncoderConfig {
                profile: None,
                idr_period: None,
            }),
            640,
            480,
            1280,
            720,
        );

        let encoder = Encoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        // 複数フレームをエンコード（in-flight フレーム有りの reconfigure をテスト）
        let frame_640x480 = create_black_frame(640, 480);
        for i in 0..3u32 {
            encoder
                .encode(
                    &frame_640x480,
                    &EncodeOptions {
                        force_intra: false,
                        force_idr: i == 0,
                        output_spspps: i == 0,
                    },
                    i,
                )
                .expect("failed to encode frame");
        }

        // エンコード中に reconfigure を発行（パイプライン競合を検証）
        encoder
            .reconfigure(ReconfigureParams {
                width: Some(1280),
                height: Some(720),
                ..Default::default()
            })
            .expect("failed to reconfigure during encoding");

        // 新解像度でエンコード継続
        let frame_1280x720 = create_black_frame(1280, 720);
        for i in 3..5u32 {
            encoder
                .encode(
                    &frame_1280x720,
                    &EncodeOptions {
                        force_intra: false,
                        force_idr: true,
                        output_spspps: true,
                    },
                    i,
                )
                .expect("failed to encode frame");
        }

        encoder.flush().expect("flush failed");
        drop(encoder);

        let frames: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            frames.len(),
            5,
            "Expected 5 encoded frames, got {}",
            frames.len()
        );

        let user_data_values: Vec<u32> = frames
            .iter()
            .map(|f| f.as_ref().expect("Frame should be Ok").user_data)
            .collect();

        // 全 5 フレームの user_data が受信されていることを確認
        for expected in 0..5u32 {
            assert!(
                user_data_values.contains(&expected),
                "Missing frame with user_data={}",
                expected
            );
        }
    }

    #[test]
    fn test_encode_after_worker_terminated() {
        use std::mem::ManuallyDrop;

        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));

        let mut encoder = ManuallyDrop::new(
            Encoder::new(config, move |frame| {
                let _ = tx.send(frame);
            })
            .unwrap(),
        );

        unsafe { ManuallyDrop::drop(&mut encoder) };

        let result = encoder.encode(
            &[],
            &EncodeOptions {
                force_intra: false,
                force_idr: false,
                output_spspps: false,
            },
            (),
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "encode() failed: encoder worker thread has terminated"
        );

        unsafe {
            ManuallyDrop::drop(&mut encoder);
        }
    }

    #[test]
    fn test_flush_after_encoder_worker_terminated() {
        use std::mem::ManuallyDrop;

        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));

        let mut encoder = ManuallyDrop::new(
            Encoder::new(config, move |frame| {
                let _ = tx.send(frame);
            })
            .unwrap(),
        );

        unsafe { ManuallyDrop::drop(&mut encoder) };

        let result = encoder.flush();
        assert_eq!(
            result.unwrap_err().to_string(),
            "flush() failed: send failed"
        );

        unsafe {
            ManuallyDrop::drop(&mut encoder);
        }
    }

    #[test]
    fn test_reconfigure_after_encoder_worker_terminated() {
        use std::mem::ManuallyDrop;

        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));

        let mut encoder = ManuallyDrop::new(
            Encoder::new(config, move |frame| {
                let _ = tx.send(frame);
            })
            .unwrap(),
        );

        unsafe { ManuallyDrop::drop(&mut encoder) };

        let result = encoder.reconfigure(ReconfigureParams::default());
        assert_eq!(
            result.unwrap_err().to_string(),
            "reconfigure() failed: send failed"
        );

        unsafe {
            ManuallyDrop::drop(&mut encoder);
        }
    }

    /// drain スレッドによって callback が遅延なく発火することを確認する
    ///
    /// worker スレッドはフレーム送信後に drain スレッドへ
    /// drain リクエストを送信し、drain スレッドが nvEncLockBitstream を
    /// ブロッキング実行する。encode() 呼び出し後に drain スレッドが
    /// 処理を完了できるだけの時間があれば、callback は次の encode() を
    /// 待たずに発火する。
    ///
    /// 本テストでは frame_interval_p = 0（n_encoder_buffer = 3）とし、
    /// 30fps 相当のフレーム間隔（33ms）で encode() を 4 回呼び出す。
    /// 各 encode() の前に encode_count をインクリメントし、
    /// 最初に発火した callback の時点での encode_count を
    /// first_cb_after に記録する。
    ///
    /// 期待値: first_cb_after < 3
    ///   最初の callback が 3 回目の encode() を待たずに発火することを確認する。
    ///   33ms の sleep で drain スレッドに十分な処理時間を与えているため、
    ///   フレーム送信後すぐに callback が呼ばれれば 1 や 2 になる。
    #[test]
    fn test_drain_thread_callback_immediate() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let mut config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));
        config.frame_interval_p = 0;

        let width = config.width;
        let height = config.height;
        let n_encoder_buffer = config.frame_interval_p as usize + 3;

        // encode_count: encode() が呼ばれるたびにメインスレッドでインクリメント
        // first_cb_after: 最初の callback 発火時点の encode_count を記録。
        //   compare_exchange により最初の callback だけが書き込む。
        let encode_count = Arc::new(AtomicUsize::new(0));
        let first_cb_after = Arc::new(AtomicUsize::new(0));
        let (cb_tx, _cb_rx) =
            mpsc::sync_channel::<Result<EncodedFrame<u32>, Error>>(n_encoder_buffer.max(8));

        let ec = encode_count.clone();
        let fca = first_cb_after.clone();
        let encoder = Encoder::new(config, move |frame| {
            let count = ec.load(Ordering::SeqCst);
            fca.compare_exchange(0, count, Ordering::SeqCst, Ordering::SeqCst)
                .ok();
            let _ = cb_tx.send(frame);
        })
        .expect("failed to create h264 encoder");

        let frame_data = create_black_frame(width, height);
        let opts = EncodeOptions {
            force_intra: false,
            force_idr: false,
            output_spspps: false,
        };

        // 30fps 相当のフレーム間隔で送信。
        // sleep により drain スレッドが encode() の間に
        // nvEncLockBitstream を完了するための十分な時間を与える。
        let frame_interval = Duration::from_millis(33);

        for i in 0..4u32 {
            encode_count.fetch_add(1, Ordering::SeqCst);
            encoder.encode(&frame_data, &opts, i).unwrap();
            std::thread::sleep(frame_interval);
        }

        // flush により未 drain の全フレームを drain し、
        // すべての callback が発火したことを保証する
        encoder.flush().unwrap();
        drop(encoder);

        // 最初の callback 発火時点の encode_count が 3 未満であることを確認。
        // drain スレッドが encode() の間に drain を完了できれば、
        // callback は次の encode() を待たずに発火する。
        let got = first_cb_after.load(Ordering::SeqCst);
        assert!(
            got < 3,
            "expected first callback before 3 encodes, got {}",
            got
        );
    }

    #[test]
    fn test_get_sequence_params_after_encoder_worker_terminated() {
        use std::mem::ManuallyDrop;

        let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
        let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
            profile: None,
            idr_period: None,
        }));

        let mut encoder = ManuallyDrop::new(
            Encoder::new(config, move |frame| {
                let _ = tx.send(frame);
            })
            .unwrap(),
        );

        unsafe { ManuallyDrop::drop(&mut encoder) };

        let result = encoder.get_sequence_params();
        assert_eq!(
            result.unwrap_err().to_string(),
            "get_sequence_params() failed: send failed"
        );

        unsafe {
            ManuallyDrop::drop(&mut encoder);
        }
    }
}
