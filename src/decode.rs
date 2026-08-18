use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

use crate::{CudaLibrary, Error, stats::Counter, sys};

/// デコーダのケーパビリティ情報
#[derive(Debug, Clone)]
pub struct DecoderCaps {
    /// コーデックがサポートされているか
    pub is_supported: bool,
    /// 最大デコード幅
    pub max_width: u32,
    /// 最大デコード高さ
    pub max_height: u32,
    /// 最大マクロブロック数
    pub max_mb_count: u32,
    /// 最小デコード幅
    pub min_width: u32,
    /// 最小デコード高さ
    pub min_height: u32,
}

/// デコーダーの統計値
///
/// `clone()` は各フィールドを個別にコピーするため、フィールド間の一貫性は保証されない。
#[derive(Debug, Clone, Default)]
pub struct DecoderStats {
    /// cuvidCreateDecoder の通算成功回数 (初回の create を含む)
    pub total_create_decoder_count: Counter,

    /// cuvidReconfigureDecoder の通算成功回数
    /// (reconfigure 経路は 0024 マージ後に導入予定のため、それまでは常に 0 を返す)
    pub total_reconfigure_decoder_count: Counter,

    /// cuvidReconfigureDecoder 呼び出しの通算失敗回数
    /// (解像度上限超過の事前検証エラーや cuvidCreateDecoder の失敗は含まない。
    ///  reconfigure 経路は 0024 マージ後に導入予定のため、それまでは常に 0 を返す)
    pub total_reconfigure_failure_count: Counter,

    /// decode() で正常に送信された通算回数
    pub total_decode_count: Counter,

    /// シーケンスコールバックの通算回数 (初回のシーケンス処理を含む)
    pub total_sequence_callback_count: Counter,

    /// デコードコールバックの通算回数 (cuvidDecodePicture の呼び出し試行回数)
    pub total_decode_callback_count: Counter,

    /// 出力フレーム数 (表示コールバックの通算回数)
    pub total_output_frame_count: Counter,
}

impl DecoderStats {
    /// 入力されたがまだ出力されていないフレーム数 (in-flight 相当) を返す
    ///
    /// `total_decode_count - total_output_frame_count` で算出する。
    /// 各カウンターは個別に読み取られるため近似値であり、`decode()` は
    /// 1 回の呼び出しに複数フレームを渡せるためバッファ内の実フレーム数とは
    /// 一致しない。デコードエラー等で出力されなかったフレームがあると
    /// 0 に戻らないことがある。
    pub fn in_flight_frames(&self) -> u64 {
        // 出力フレーム数は入力フレーム数を超えないため通常は負数にならないが、
        // 2 つのカウンターの読み取りは原子的でないため saturating で算出する
        self.total_decode_count
            .get()
            .saturating_sub(self.total_output_frame_count.get())
    }
}

/// デコーダー用コーデック識別子
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderCodec {
    /// H.264
    H264,
    /// HEVC
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

/// デコーダー出力サーフェスフォーマット (NVDEC: cudaVideoSurfaceFormat)
///
/// 現在はフレームコピー処理が NV12 前提のため、NV12 のみサポートしている。
/// 他フォーマットが必要になった場合は、コピー処理の分岐と DecodedFrame の拡張を同時に行うこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceFormat {
    /// Semi-Planar YUV 4:2:0 8bit [Y plane + interleaved UV plane]
    Nv12,
}

impl SurfaceFormat {
    fn to_sys(self) -> u32 {
        match self {
            SurfaceFormat::Nv12 => sys::cudaVideoSurfaceFormat_enum_cudaVideoSurfaceFormat_NV12,
        }
    }
}

/// デコーダーの設定
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    /// コーデック識別子
    pub codec: DecoderCodec,

    /// 使用する GPU デバイスの ID
    pub device_id: i32,

    /// デコードサーフェス数の上限
    ///
    /// `0` は指定できず、`Decoder::new` が設定エラーとして拒否する。
    /// parser が正しいデコードに必要な最小サーフェス数
    /// (`CUVIDEOFORMAT.min_num_decode_surfaces`) がこの上限を超える場合は、
    /// `decode()` 中の sequence callback で既存 decoder を破棄する前にエラーを返す。
    /// 実際に割り当てられるサーフェス数は常にこの上限以下になる。
    pub max_num_decode_surfaces: u32,

    /// 表示遅延 (0 = 低遅延)
    pub max_display_delay: u32,

    /// 出力サーフェスフォーマット (NVDEC: OutputFormat)
    pub surface_format: SurfaceFormat,
}

struct DecoderState {
    lib: CudaLibrary,
    ctx: sys::CUcontext,
    ctx_lock: sys::CUvideoctxlock,
    parser: sys::CUvideoparser,
    decoder: sys::CUvideodecoder,
    // 利用側が指定したデコードサーフェス数の上限
    // (sequence callback で parser が要求する最小サーフェス数を検証するために保持する)
    max_num_decode_surfaces: u32,
    width: u32,
    height: u32,
    // display_area の原点 (left / top)。
    // 非ゼロの場合、mapped output surface 上の表示領域はこの位置から始まるため、
    // コピー元オフセットの計算に使う。
    display_area_left: u32,
    display_area_top: u32,
    surface_width: u32,
    surface_height: u32,
    surface_format: u32,
    frame_tx: Sender<RawFrame>,
    frame_rx: Receiver<RawFrame>,
    // 読み書きはワーカースレッドのみ（パーサーコールバックも同スレッド）なので Mutex 不要
    callback_error: Option<Error>,
    stats: Arc<DecoderStats>,
}

unsafe impl Send for DecoderState {}

impl DecoderState {
    /// 指定されたコーデック設定でデコーダーインスタンスを生成する
    fn new(config: DecoderConfig) -> Result<Box<Self>, Error> {
        let codec_type = match config.codec {
            DecoderCodec::H264 => sys::cudaVideoCodec_enum_cudaVideoCodec_H264,
            DecoderCodec::Hevc => sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC,
            DecoderCodec::Av1 => sys::cudaVideoCodec_enum_cudaVideoCodec_AV1,
            DecoderCodec::Vp8 => sys::cudaVideoCodec_enum_cudaVideoCodec_VP8,
            DecoderCodec::Vp9 => sys::cudaVideoCodec_enum_cudaVideoCodec_VP9,
            DecoderCodec::Jpeg => sys::cudaVideoCodec_enum_cudaVideoCodec_JPEG,
        };
        Self::new_with_codec(codec_type, config)
    }

    /// 指定コーデックのデコーダのケーパビリティをクエリする
    fn query_caps(codec: DecoderCodec, device_id: i32) -> Result<DecoderCaps, Error> {
        let codec_type = match codec {
            DecoderCodec::H264 => sys::cudaVideoCodec_enum_cudaVideoCodec_H264,
            DecoderCodec::Hevc => sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC,
            DecoderCodec::Av1 => sys::cudaVideoCodec_enum_cudaVideoCodec_AV1,
            DecoderCodec::Vp8 => sys::cudaVideoCodec_enum_cudaVideoCodec_VP8,
            DecoderCodec::Vp9 => sys::cudaVideoCodec_enum_cudaVideoCodec_VP9,
            DecoderCodec::Jpeg => sys::cudaVideoCodec_enum_cudaVideoCodec_JPEG,
        };
        Self::query_caps_with_codec(device_id, codec_type)
    }

    fn query_caps_with_codec(
        device_id: i32,
        codec_type: sys::cudaVideoCodec,
    ) -> Result<DecoderCaps, Error> {
        unsafe {
            let lib = CudaLibrary::load()?;

            // 一時的な CUDA コンテキストを作成
            let mut ctx = std::ptr::null_mut();
            lib.cu_ctx_create(&mut ctx, 0, device_id)?;

            let lib_clone = lib.clone();
            let ctx_guard = crate::ReleaseGuard::new(move || {
                let _ = lib_clone.cu_ctx_destroy(ctx);
            });

            let caps = lib.with_context(ctx, || {
                let mut decode_caps: sys::CUVIDDECODECAPS = std::mem::zeroed();
                decode_caps.eCodecType = codec_type;
                decode_caps.eChromaFormat =
                    sys::cudaVideoChromaFormat_enum_cudaVideoChromaFormat_420;
                decode_caps.nBitDepthMinus8 = 0;

                lib.cuvid_get_decoder_caps(&mut decode_caps)?;

                Ok(DecoderCaps {
                    is_supported: decode_caps.bIsSupported != 0,
                    max_width: decode_caps.nMaxWidth,
                    max_height: decode_caps.nMaxHeight,
                    max_mb_count: decode_caps.nMaxMBCount,
                    min_width: decode_caps.nMinWidth as u32,
                    min_height: decode_caps.nMinHeight as u32,
                })
            })?;

            ctx_guard.cancel();
            lib.cu_ctx_destroy(ctx)?;

            Ok(caps)
        }
    }

    fn new_with_codec(
        codec_type: sys::cudaVideoCodec,
        config: DecoderConfig,
    ) -> Result<Box<Self>, Error> {
        // デコードサーフェス数の上限として 0 は不正な設定なので拒否する
        // (0 では実効サーフェス数を決定できないため)
        if config.max_num_decode_surfaces == 0 {
            return Err(Error::new_custom(
                "Decoder::new",
                "max_num_decode_surfaces must be greater than 0",
            ));
        }

        unsafe {
            let lib = CudaLibrary::load()?;

            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let ctx_flags = 0; // デフォルトのコンテキストフラグ
            lib.cu_ctx_create(&mut ctx, ctx_flags, config.device_id)?;

            let ctx_guard = crate::ReleaseGuard::new(|| {
                let _ = lib.cu_ctx_destroy(ctx);
            });

            // デコーダー用のコンテキストロックを作成
            let mut ctx_lock = ptr::null_mut();
            lib.cuvid_ctx_lock_create(&mut ctx_lock, ctx)?;

            let ctx_lock_guard = crate::ReleaseGuard::new(|| {
                let _ = lib.cuvid_ctx_lock_destroy(ctx_lock);
            });

            // チャンネルを作成
            let (frame_tx, frame_rx) = mpsc::channel();

            // デコーダーの状態を作成
            let mut state = Box::new(DecoderState {
                lib: lib.clone(),
                ctx,
                ctx_lock,
                parser: ptr::null_mut(),
                decoder: ptr::null_mut(),
                max_num_decode_surfaces: config.max_num_decode_surfaces,
                width: 0,
                height: 0,
                display_area_left: 0,
                display_area_top: 0,
                surface_width: 0,
                surface_height: 0,
                surface_format: config.surface_format.to_sys(),
                frame_tx,
                frame_rx,
                callback_error: None,
                stats: Arc::new(DecoderStats::default()),
            });

            // 映像パーサーを作成する
            let mut parser_params: sys::CUVIDPARSERPARAMS = std::mem::zeroed();
            parser_params.CodecType = codec_type;
            // NVDEC Video Decoder API Programming Guide 13.0「4.1.1. Creating a parser」に従い、
            // sequence header を解析する前の仮値として 1 を設定する。
            // 実際のサーフェス数は sequence callback の戻り値 (実効サーフェス数) で
            // parser の DPB 数を上書きして決定する。
            // なお、sequence callback の戻り値が 1 の場合は parser の DPB 数を更新しないため、
            // 仮値と実際のサーフェス数を一致させる必要がある場合は実効サーフェス数を 1 に保つ。
            parser_params.ulMaxNumDecodeSurfaces = 1;
            parser_params.ulMaxDisplayDelay = config.max_display_delay;
            parser_params.pUserData = state.as_mut() as *const _ as *mut c_void;
            parser_params.pfnSequenceCallback = Some(handle_video_sequence);
            parser_params.pfnDecodePicture = Some(handle_picture_decode);
            parser_params.pfnDisplayPicture = Some(handle_picture_display);

            let mut parser = ptr::null_mut();
            lib.cuvid_create_video_parser(&mut parser, &mut parser_params)?;

            // parser を state に保存する
            state.parser = parser;

            // 成功したのでクリーンアップをキャンセル
            ctx_guard.cancel();
            ctx_lock_guard.cancel();

            Ok(state)
        }
    }

    /// parser と decoder の両方へ適用する実効サーフェス数を決定する
    ///
    /// `min` は `CUVIDEOFORMAT.min_num_decode_surfaces`、`max` は
    /// `DecoderConfig.max_num_decode_surfaces` を表す。
    ///
    /// `min_num_decode_surfaces` は正しいデコードに必要な最小サーフェス数であり、
    /// `third_party/nvcodec/include/nvcuvid.h` の `CUVIDEOFORMAT` と
    /// `PFNVIDSEQUENCECALLBACK` が根拠である。
    ///
    /// 戻り値は呼び出し側で `sequence callback` の戻り値や `ulNumDecodeSurfaces` に
    /// 使われる。戻り値は常に u8 の上限 (255) 以下になるため、`as i32` への
    /// キャストは安全である (実効サーフェス数は `min` か 2 か 1 のいずれかで、
    /// `min` は `CUVIDEOFORMAT.min_num_decode_surfaces` 由来のため 255 を超えない)。
    ///
    /// - `PFNVIDSEQUENCECALLBACK` の戻り値が 1 の場合は parser の DPB 数を更新しないため、
    ///   戻り値で parser の DPB 数を上書きするには 2 以上を返す必要がある
    /// - そのため、`min == 1` でも `max >= 2` なら 2 を使って parser の DPB 数を確定させる
    fn determine_num_decode_surfaces(min: u32, max: u32) -> Result<u32, Error> {
        if min == 0 {
            return Err(Error::new_custom(
                "determine_num_decode_surfaces",
                "min_num_decode_surfaces must be greater than 0",
            ));
        }
        if min > max {
            return Err(Error::new_custom_owned(
                "determine_num_decode_surfaces",
                format!("min_num_decode_surfaces ({min}) exceeds max_num_decode_surfaces ({max})"),
            ));
        }
        // PFNVIDSEQUENCECALLBACK の戻り値が 2 以上の場合のみ parser の DPB 数を更新できる
        if min >= 2 {
            // 最小値が 2 以上なら、そのまま使えば parser の DPB 数も同じ値に更新される
            Ok(min)
        } else if max >= 2 {
            // 最小値が 1 なら、parser の DPB 数を確実に更新できる最小値として 2 を使う
            Ok(2)
        } else {
            // 最小値も上限も 1 なら、parser の初期値と同じ 1 を使う
            Ok(1)
        }
    }

    /// 圧縮された映像フレームをデコードする
    ///
    /// 内部でコールバックが失敗した場合、その具体的エラーが返る。
    /// エラー後のデコーダー停止 (終端状態への遷移) は呼び出し側 (`DecodeWorker::run`) の責務である。
    fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        // [NOTE]
        // cuvidParseVideoData は内部でデータをコピーまたは即座に処理するため、
        // このメソッドの呼び出し直後に data を破棄しても安全
        unsafe {
            let mut packet: sys::CUVIDSOURCEDATAPACKET = std::mem::zeroed();
            packet.payload = data.as_ptr();
            packet.payload_size = data.len() as u64;
            // １回のデコードごとに１枚の映像が生成されるはずなので
            // CUVID_PKT_ENDOFPICTURE を指定する
            packet.flags = sys::CUvideopacketflags_CUVID_PKT_ENDOFPICTURE as u64;
            packet.timestamp = 0;

            let parse_result = self.lib.cuvid_parse_video_data(self.parser, &mut packet);
            // コールバックが callback_error フィールドに格納した具体的エラーがあれば、
            // cuvidParseVideoData が返す汎用 CUDA エラーより優先して通知する。
            // コールバックは callback_error 格納後も失敗 (0) を返し続けるため、通常は両方失敗する。
            self.prefer_callback_error(parse_result)
        }
    }

    /// EOS を送り、残っているデコード処理の完了を待つ
    ///
    /// コールバック失敗時は `callback_error` を優先して返す（[`DecoderState::decode`] と同じ）。
    fn send_eos(&mut self) -> Result<(), Error> {
        unsafe {
            // EOS をデコーダーに伝える
            let mut packet: sys::CUVIDSOURCEDATAPACKET = std::mem::zeroed();
            packet.payload = ptr::null();
            packet.payload_size = 0;
            packet.flags = sys::CUvideopacketflags_CUVID_PKT_ENDOFSTREAM as u64;
            packet.timestamp = 0;

            let parse_result = self.lib.cuvid_parse_video_data(self.parser, &mut packet);
            self.prefer_callback_error(parse_result)?;

            // パーサーは非同期でデータを処理するので、
            // すべてのデコード操作が完了するまでここで待機（同期）する
            self.lib
                .with_context(self.ctx, || self.lib.cu_ctx_synchronize())?;
        }
        Ok(())
    }

    fn prefer_callback_error<T>(&mut self, result: Result<T, Error>) -> Result<T, Error> {
        if let Some(e) = self.callback_error.take() {
            Err(e)
        } else {
            result
        }
    }

    /// デコード済みのフレームを取り出す
    fn next_frame(&self) -> Option<RawFrame> {
        self.frame_rx.try_recv().ok()
    }

    /// シーケンスコールバック処理 (pfnSequenceCallback から呼ばれる)
    ///
    /// 既存デコーダーがあれば破棄し、現在のシーケンスのフォーマットで再作成する。
    /// また、表示領域の検証と width / height / surface サイズの更新を行う。
    /// 成功時はデコードサーフェス数を返し、失敗時は `Err` を返す。
    /// 戻り値のデコードサーフェス数は extern "C" ラッパー経由で parser へ渡され、
    /// parser はこの値で `CUVIDPICPARAMS.CurrPicIdx` を割り当てる。
    fn handle_video_sequence(&mut self, format: &sys::CUVIDEOFORMAT) -> Result<i32, Error> {
        self.stats.total_sequence_callback_count.inc();
        // 実効サーフェス数を決定し、上限不足や不正な値を既存デコーダーを破棄する前に検証する
        let num_decode_surfaces = Self::determine_num_decode_surfaces(
            format.min_num_decode_surfaces as u32,
            self.max_num_decode_surfaces,
        )?;
        // display_area は signed 整数のため、壊れたストリームで負値になる可能性がある
        // 既存デコーダーを破棄する前に検証し、不正な場合は fail-fast で終端させる
        let left = format.display_area.left;
        let right = format.display_area.right;
        let top = format.display_area.top;
        let bottom = format.display_area.bottom;
        if left < 0
            || top < 0
            || right <= left
            || bottom <= top
            || right as u32 > format.coded_width
            || bottom as u32 > format.coded_height
        {
            return Err(Error::new_custom(
                "handle_video_sequence",
                "invalid display_area in video format",
            ));
        }
        // デコーダーが既に作成されている場合は破棄して再作成する
        // ストリーム中の解像度変更に対応するため
        if !self.decoder.is_null() {
            self.lib
                .with_context(self.ctx, || self.lib.cuvid_destroy_decoder(self.decoder))?;
            self.decoder = ptr::null_mut();
        }

        // デコーダーの作成情報を設定
        let mut create_info: sys::CUVIDDECODECREATEINFO = unsafe { std::mem::zeroed() };
        create_info.CodecType = format.codec;
        create_info.ChromaFormat = format.chroma_format;
        create_info.OutputFormat = self.surface_format;
        create_info.bitDepthMinus8 = format.bit_depth_luma_minus8 as u64;
        create_info.DeinterlaceMode = if format.progressive_sequence != 0 {
            sys::cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Weave
        } else {
            sys::cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Adaptive
        };
        create_info.ulNumOutputSurfaces = 2; // 出力サーフェスの数（ダブルバッファリング用に 2 を指定）
        create_info.ulCreationFlags =
            sys::cudaVideoCreateFlags_enum_cudaVideoCreate_PreferCUVID as u64; // CUVID ハードウェアデコーダーの使用を優先するフラグ
        create_info.ulNumDecodeSurfaces = num_decode_surfaces as u64;
        create_info.ulWidth = format.coded_width as u64;
        create_info.ulHeight = format.coded_height as u64;
        create_info.ulMaxWidth = format.coded_width as u64;
        create_info.ulMaxHeight = format.coded_height as u64;
        create_info.ulTargetWidth = format.coded_width as u64;
        create_info.ulTargetHeight = format.coded_height as u64;

        // パーサーと共有するコンテキストロックを使用
        create_info.vidLock = self.ctx_lock;

        self.lib.with_context(self.ctx, || {
            self.lib
                .cuvid_create_decoder(&mut self.decoder, &mut create_info)
        })?;
        self.stats.total_create_decoder_count.inc();
        self.width = (right - left) as u32;
        self.height = (bottom - top) as u32;
        // display_area の原点を保持する。非ゼロの場合は mapped output surface 上の
        // 表示領域がこの位置から始まるため、handle_picture_display でコピー元オフセットとして使う。
        // 検証済みのため負値にはならない。
        self.display_area_left = left as u32;
        self.display_area_top = top as u32;
        self.surface_width = format.coded_width;
        self.surface_height = format.coded_height;

        // シーケンスコールバックの戻り値は decoder の ulNumDecodeSurfaces と同じ値にする
        // (parser がこの値で CUVIDPICPARAMS.CurrPicIdx を割り当てるため、両者を一致させる)
        Ok(num_decode_surfaces as i32)
    }

    /// ピクチャデコードコールバック処理 (pfnDecodePicture から呼ばれる)
    ///
    /// 指定されたピクチャを `cuvidDecodePicture` でデコードする。
    fn handle_picture_decode(&mut self, pic_params: &sys::CUVIDPICPARAMS) -> Result<(), Error> {
        self.stats.total_decode_callback_count.inc();
        if self.decoder.is_null() {
            return Err(Error::new_custom(
                "handle_picture_decode",
                "decoder not initialized",
            ));
        }

        self.lib.with_context(self.ctx, || {
            self.lib
                .cuvid_decode_picture(self.decoder, pic_params as *const _ as *mut _)
        })?;

        Ok(())
    }

    /// ピクチャ表示コールバック処理 (pfnDisplayPicture から呼ばれる)
    ///
    /// デコード済みフレームを mapped output surface からホストメモリへコピーし、
    /// `frame_tx` チャンネル経由で出力フレームとして送信する。
    fn handle_picture_display(&self, disp_info: &sys::CUVIDPARSERDISPINFO) -> Result<(), Error> {
        if self.decoder.is_null() {
            return Err(Error::new_custom(
                "handle_picture_display",
                "decoder not initialized",
            ));
        }

        let decoded_frame = self.lib.with_context(self.ctx, || unsafe {
            // ビデオ処理パラメーターを設定
            let mut proc_params: sys::CUVIDPROCPARAMS = std::mem::zeroed();
            proc_params.progressive_frame = disp_info.progressive_frame;
            proc_params.top_field_first = disp_info.top_field_first;
            proc_params.second_field = disp_info.repeat_first_field + 1;
            proc_params.output_stream = ptr::null_mut();

            // デコード済みフレームをマップ
            let mut device_ptr = 0u64;
            let mut pitch = 0u32;
            self.lib.cuvid_map_video_frame(
                self.decoder,
                disp_info.picture_index,
                &mut device_ptr,
                &mut pitch,
                &mut proc_params,
            )?;

            // 確実にフレームをアンマップするためのガードを作成
            let _unmap_guard = crate::ReleaseGuard::new(|| {
                let _ = self.lib.cuvid_unmap_video_frame(self.decoder, device_ptr);
            });

            // フレームサイズを計算 (NV12 形式: Y プレーン + UV プレーン)
            // 注意: NVDEC は高さを 2 でアライメントする
            let aligned_height = (self.surface_height + 1) & !1;
            let y_size = pitch as usize * self.height as usize;
            let uv_size = pitch as usize * (self.height as usize).div_ceil(2);
            let frame_size = y_size + uv_size;

            // フレーム用のホストメモリを割り当て (Y プレーン + UV プレーン、各プレーン内は pitch ストライド)
            let mut host_data = vec![0u8; frame_size];

            // display_area の原点が非ゼロの場合、表示領域は mapped output surface の
            // 左上ではなく display_area の位置から始まる。公開する寸法 (width / height) は
            // 表示領域の寸法なので、コピー元も表示領域の左上に合わせる。
            //
            // 1D 連続コピーだと、各行の行末パディングが次行や UV プレーンへ食い込む。
            // 特に非ゼロ left かつ表示下端が coded いっぱい (bottom == aligned_height) のとき、
            // Y コピーの末尾が UV プレーン開始位置を越えたり、UV コピーの末尾がマッピング外へ
            // 伸びたりする。そこで cuMemcpy2D を使い、行矩形 (WidthInBytes だけ) を
            // 表示領域の行数分だけコピーして、パディングを含めない。
            //
            // - Y プレーン: 表示領域の top 行目 * pitch + left 列目から、width バイト x height 行
            // - UV プレーン: NV12 の 2x2 クロマサブサンプリングのため、mapped output surface の
            //   UV 開始位置 (coded 高さを 2 でアラインした位置) から top / 2 行目、
            //   left / 2 画素分 (left バイト) 進めた位置から、width バイト x height.div_ceil(2) 行
            let width = self.width as usize;
            let height = self.height as usize;
            let y_src_y = self.display_area_top as usize;
            let y_src_x = self.display_area_left as usize;
            let uv_src_y = aligned_height as usize + (self.display_area_top as usize / 2);
            let uv_src_x = self.display_area_left as usize; // left / 2 画素 x 2 バイト

            // Y プレーンをコピー
            let mut y_copy: sys::CUDA_MEMCPY2D = std::mem::zeroed();
            y_copy.srcMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE;
            y_copy.srcDevice = device_ptr;
            y_copy.srcPitch = pitch as usize;
            y_copy.srcY = y_src_y;
            y_copy.srcXInBytes = y_src_x;
            y_copy.dstMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_HOST;
            y_copy.dstHost = host_data.as_mut_ptr() as *mut c_void;
            y_copy.dstPitch = pitch as usize;
            y_copy.WidthInBytes = width;
            y_copy.Height = height;
            self.lib.cu_memcpy_2d(&y_copy)?;

            // UV プレーンをコピー
            let mut uv_copy: sys::CUDA_MEMCPY2D = std::mem::zeroed();
            uv_copy.srcMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE;
            uv_copy.srcDevice = device_ptr;
            uv_copy.srcPitch = pitch as usize;
            uv_copy.srcY = uv_src_y;
            uv_copy.srcXInBytes = uv_src_x;
            uv_copy.dstMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_HOST;
            uv_copy.dstHost = host_data[y_size..].as_mut_ptr() as *mut c_void;
            uv_copy.dstPitch = pitch as usize;
            uv_copy.WidthInBytes = width;
            uv_copy.Height = height.div_ceil(2);
            self.lib.cu_memcpy_2d(&uv_copy)?;

            // デコード済みフレームを作成
            Ok(RawFrame {
                width: self.width,
                height: self.height,
                pitch: pitch as usize,
                data: host_data,
            })
        })?;

        self.stats.total_output_frame_count.inc();
        // チャンネル経由で送信 (受信側が破棄されている場合の送信エラーは無視)
        let _ = self.frame_tx.send(decoded_frame);

        Ok(())
    }
}

impl Drop for DecoderState {
    fn drop(&mut self) {
        if !self.parser.is_null() {
            let _ = self.lib.cuvid_destroy_video_parser(self.parser);
        }

        if !self.decoder.is_null() {
            let _ = self
                .lib
                .with_context(self.ctx, || self.lib.cuvid_destroy_decoder(self.decoder));
        }

        if !self.ctx_lock.is_null() {
            let _ = self.lib.cuvid_ctx_lock_destroy(self.ctx_lock);
        }

        if !self.ctx.is_null() {
            let _ = self.lib.cu_ctx_destroy(self.ctx);
        }
    }
}

enum Job<T> {
    Decode { data: Vec<u8>, user_data: T },
    Flush { done: SyncSender<()> },
    Terminate,
}

/// デコード結果を通知するためのハンドラー
///
/// デコード処理が完了するたびに [`DecodeHandler::on_decoded`] が呼ばれる。
/// 一度 [`DecodeHandler::on_decoded`] に `Err` を渡したら、そのデコーダーは終端状態になる。
/// 終端状態とは、それ以降はデコードせず、送信されたデータに対して終端を表す `Err` を
/// コールバックするだけの状態である。
/// 最初の `Err` で [`Decoder`] を捨て、復旧は新しいインスタンスを作ること。
///
/// 終端時点で出力待ちだったフレームにコールバックは呼び出されない。
/// 終端後に送った [`Decoder::decode`] には、`Err` が渡される。
pub trait DecodeHandler: Send + 'static {
    /// ユーザーデータ型
    type UserData: Send + 'static;
    /// エラー型
    type Error: From<crate::Error> + Send + 'static;
    /// デコード完了時に呼ばれる
    fn on_decoded(&mut self, result: Result<DecodedFrame<Self::UserData>, Self::Error>);
}

/// `FnMut` クロージャを [`DecodeHandler`] にするラッパー
pub struct FnDecodeHandler<T, E = crate::Error> {
    f: Box<dyn FnMut(Result<DecodedFrame<T>, E>) + Send + 'static>,
}

impl<T, E> FnDecodeHandler<T, E> {
    /// `FnMut` クロージャから [`FnDecodeHandler`] を生成する
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(Result<DecodedFrame<T>, E>) + Send + 'static,
    {
        Self { f: Box::new(f) }
    }
}

impl<T, E> DecodeHandler for FnDecodeHandler<T, E>
where
    T: Send + 'static,
    E: From<crate::Error> + Send + 'static,
{
    type UserData = T;
    type Error = E;
    fn on_decoded(&mut self, result: Result<DecodedFrame<T>, E>) {
        (self.f)(result);
    }
}

/// デコーダー
///
/// 内部で専用のワーカースレッドを起動し、非同期でデコードを行う。
/// デコードが完了すると、コンストラクタで渡したハンドラがワーカースレッド上で即座に呼び出される。
///
/// このインスタンスは一度 [`DecodeHandler::on_decoded`] に `Err` を渡した時点で終端状態になる。
/// 終端状態とは、それ以降はデコードせず、送信されたデータに対して終端を表す `Err` を
/// コールバックするだけの状態である。
///
/// 終端時点で出力待ちだったフレームにはコールバックが呼び出されない (捨てられる)。
/// 終端後の [`Decoder::decode`] は送信は成功するが、内部デコードはせず、
/// その分について終端を表す `Err` がコールバックされる。
///
/// 終端後のこのインスタンスは復旧できない。最初の `Err` でこのインスタンスを捨て、
/// 新しい `Decoder` を作り直すこと。
pub struct Decoder<H: DecodeHandler> {
    job_tx: SyncSender<Job<H::UserData>>,
    worker: Option<JoinHandle<()>>,
    stats: Arc<DecoderStats>,
}

impl<H: DecodeHandler> Decoder<H> {
    /// デコーダーを生成し、内部ワーカースレッドを起動する
    pub fn new(config: DecoderConfig, handler: H) -> Result<Self, Error> {
        let (job_tx, job_rx) = mpsc::sync_channel::<Job<H::UserData>>(4);

        let state = DecoderState::new(config)?;

        let stats = state.stats.clone();

        let worker = std::thread::Builder::new()
            .name("nvcodec-decoder".into())
            .spawn(move || {
                DecodeWorker::run(state, handler, job_rx);
            })
            .map_err(|_e| Error::new_custom("Decoder::new", "failed to spawn decoder thread"))?;

        Ok(Self {
            job_tx,
            worker: Some(worker),
            stats,
        })
    }

    /// デコーダーの統計値を取得する
    ///
    /// 返される参照は共有統計値への参照であり、読み出すたびに
    /// 最新値が読める。保持したい場合は `clone()` でスナップショットを取得する。
    pub fn stats(&self) -> &DecoderStats {
        &self.stats
    }

    /// 圧縮された映像フレームをデコードする
    ///
    /// フレームデータとユーザーデータをワーカースレッドに送信し、即座に戻る。
    /// デコードが完了すると、コンストラクタで渡したコールバックハンドラが呼び出される。
    ///
    /// # 注意
    ///
    /// デコーダーが終端状態でも送信は成功するが、常に
    /// [`DecodeHandler::on_decoded`] には `Err` が渡される。
    /// なお、`decode` の呼び出し回数と `on_decoded` の呼び出し回数は常に一致するとは限らない。
    /// 終端状態に入る前に送信済みで未出力の分にはコールバックが呼び出されない。
    pub fn decode(&self, data: &[u8], user_data: H::UserData) -> Result<(), Error> {
        self.job_tx
            .send(Job::Decode {
                data: data.to_vec(),
                user_data,
            })
            .map_err(|_| Error::new_custom("decode", "decoder worker thread has terminated"))?;
        self.stats.total_decode_count.inc();
        Ok(())
    }

    /// 送信済みの未完了フレームがすべて出力されるまで待機する
    ///
    /// デコーダーが終端状態でない場合、送信済みの未完了フレームがすべて出力され、
    /// そのコールバックがすべて呼び出されたあとで戻る。
    /// このメソッド呼び出し後もデコードは継続できる。
    ///
    /// デコーダーが終端状態の場合 (このメソッドの呼び出し中に終端状態に遷移した場合を含む) は、
    /// 未完了フレームの出力を待たずに戻る。
    ///
    /// このメソッドの呼び出し中に終端状態に遷移した場合は、
    /// その原因 `Err` が [`DecodeHandler::on_decoded`] に渡されたあとで、
    /// 呼び出し元に処理が戻る。
    pub fn flush(&self) -> Result<(), Error> {
        let (tx, rx) = mpsc::sync_channel(0);
        self.job_tx
            .send(Job::Flush { done: tx })
            .map_err(|_| Error::new_custom("flush", "send failed"))?;
        rx.recv()
            .map_err(|_| Error::new_custom("flush", "recv failed"))?;
        Ok(())
    }
}

impl<H: DecodeHandler> Drop for Decoder<H> {
    fn drop(&mut self) {
        let _ = self.job_tx.send(Job::Terminate);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl<H: DecodeHandler> std::fmt::Debug for Decoder<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder").finish_non_exhaustive()
    }
}

unsafe impl<H: DecodeHandler> Send for Decoder<H> {}

/// 指定コーデックのデコーダのケーパビリティをクエリする
pub fn query_decoder_caps(codec: DecoderCodec, device_id: i32) -> Result<DecoderCaps, Error> {
    DecoderState::query_caps(codec, device_id)
}

unsafe extern "C" fn handle_video_sequence(
    user_data: *mut c_void,
    format: *mut sys::CUVIDEOFORMAT,
) -> i32 {
    if user_data.is_null() || format.is_null() {
        return 0;
    }
    let state = unsafe { &mut *(user_data as *mut DecoderState) };
    match state.handle_video_sequence(unsafe { &*format }) {
        Ok(val) => val,
        Err(e) => {
            // 具体的エラーを callback_error フィールドに格納し、パーサーには失敗 (0) を伝える
            store_callback_error(state, e);
            0
        }
    }
}

unsafe extern "C" fn handle_picture_decode(
    user_data: *mut c_void,
    pic_params: *mut sys::CUVIDPICPARAMS,
) -> i32 {
    if user_data.is_null() || pic_params.is_null() {
        return 0;
    }
    let state = unsafe { &mut *(user_data as *mut DecoderState) };
    match state.handle_picture_decode(unsafe { &*pic_params }) {
        Ok(()) => 1,
        Err(e) => {
            store_callback_error(state, e);
            0
        }
    }
}

unsafe extern "C" fn handle_picture_display(
    user_data: *mut c_void,
    disp_info: *mut sys::CUVIDPARSERDISPINFO,
) -> i32 {
    if user_data.is_null() || disp_info.is_null() {
        return 0;
    }
    let state = unsafe { &mut *(user_data as *mut DecoderState) };
    match state.handle_picture_display(unsafe { &*disp_info }) {
        Ok(()) => 1,
        Err(e) => {
            store_callback_error(state, e);
            0
        }
    }
}

/// 内部用のデコード済み映像フレーム
#[derive(Debug, Clone)]
struct RawFrame {
    width: u32,
    height: u32,
    pitch: usize,
    data: Vec<u8>,
}

/// デコードされた映像フレーム (NV12 形式)
///
/// # 出力契約
///
/// フレームの寸法と画素データは、その picture の表示領域 (display area) に一致する。
///
/// - [`DecodedFrame::width`] / [`DecodedFrame::height`] は表示領域の寸法を返す
/// - [`DecodedFrame::y_plane`] は表示領域の Y データだけを返す
/// - [`DecodedFrame::uv_plane`] は表示領域の UV データだけを返す
/// - [`DecodedFrame::y_stride`] / [`DecodedFrame::uv_stride`] が返す stride と各行の配置が一致する
/// - coded サイズの padding や display area の crop が画素データに正しく反映される
///
/// display area の原点 (left / top) が非ゼロの入力でも、上記の契約を満たす。
/// コーデックや GPU / driver による実際の挙動は NVIDIA GPU 実機で確認する。
#[derive(Debug, Clone)]
pub struct DecodedFrame<T> {
    width: u32,
    height: u32,
    pitch: usize,
    data: Vec<u8>,
    user_data: T,
}

impl<T> DecodedFrame<T> {
    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        let y_size = self.pitch * self.height as usize;
        &self.data[..y_size]
    }

    /// フレームの UV 成分のデータを返す（NV12はインターリーブ形式）
    pub fn uv_plane(&self) -> &[u8] {
        let y_size = self.pitch * self.height as usize;
        let uv_size = self.pitch * (self.height as usize).div_ceil(2);
        &self.data[y_size..y_size + uv_size]
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        self.pitch
    }

    /// フレームの UV 成分のストライドを返す
    pub fn uv_stride(&self) -> usize {
        self.pitch
    }

    /// フレームの幅を返す
    ///
    /// 表示領域 (display area) の幅を返す。coded サイズではなく、crop 後の表示寸法である。
    pub fn width(&self) -> usize {
        self.width as usize
    }

    /// フレームの高さを返す
    ///
    /// 表示領域 (display area) の高さを返す。coded サイズではなく、crop 後の表示寸法である。
    pub fn height(&self) -> usize {
        self.height as usize
    }

    /// ユーザーデータを取得する
    pub fn user_data(&self) -> &T {
        &self.user_data
    }

    /// フレームデータとユーザーデータに分解する（所有権を移動）
    pub fn into_parts(self) -> (Vec<u8>, T) {
        (self.data, self.user_data)
    }
}

/// デコードワーカーのループ状態と処理をまとめる構造体
struct DecodeWorker<H: DecodeHandler> {
    state: Box<DecoderState>,
    handler: H,
    pending_user_data: VecDeque<H::UserData>,
    // 最初のデコードエラー以降、このデコーダーは終端状態になる
    terminated: bool,
}

impl<H: DecodeHandler> DecodeWorker<H> {
    /// 終端状態へ遷移する
    ///
    /// キューに残った Ok フレームを破棄し、未完了 pending をコールバックせず捨てる。
    /// 失敗した parse 由来の Ok と先行 pending を誤ペアリングしないための処理でもある。
    fn enter_terminated(&mut self) {
        self.terminated = true;
        self.discard_queued_frames();
        self.pending_user_data.clear();
    }

    fn handle_decode(&mut self, data: &[u8], user_data: H::UserData) {
        if self.terminated {
            // 終端後はデコードせず、終端エラーを通知する。
            // デコード結果の Ok フレームは以降一切来ない。
            self.handler.on_decoded(Err(terminal_decode_error().into()));
            return;
        }

        if let Err(e) = self.state.decode(data) {
            // 最初のデコードエラーで終端に入る。
            // ワーカーを return してはいけない (decode() は送信成功で Ok を返すため、
            // キュー済みジョブのコールバックが消える)。終端後もジョブを受けて応答する。
            //
            // このジョブの user_data は pending に積んでいないためここで破棄される。
            // 利用側は最初の Err で Decoder を捨てることを推奨。
            self.enter_terminated();
            self.handler.on_decoded(Err(e.into()));
            return;
        }

        self.pending_user_data.push_back(user_data);
        if !self.drain_frames() {
            // missing user data（不変条件違反）で終端に入る。
            // 原因 Err は drain_frames 内で通知済み。残りはコールバックせず捨てる。
            self.enter_terminated();
        }
    }

    fn handle_flush(&mut self, done: SyncSender<()>) {
        if !self.terminated {
            // send_eos の任意の Err（callback_error 優先の具体エラー、
            // および synchronize 失敗など）で終端する。Decode 経路と対称。
            match self.state.send_eos() {
                Ok(()) => {
                    if !self.drain_frames() {
                        // missing user data（不変条件違反）で終端に入る
                        self.enter_terminated();
                    }
                }
                Err(e) => {
                    self.enter_terminated();
                    self.handler.on_decoded(Err(e.into()));
                }
            }
        }
        let _ = done.send(());
    }

    /// 残っている非同期処理を完了させてから終了する
    ///
    /// 終端前は残フレームを drain し、終端後は残フレームを drain せず終了する。
    /// `state` の Drop はこのメソッドを呼び出した側で行われる (CUDA リソース解放)。
    fn finish(&mut self) {
        if !self.terminated {
            let _ = self.state.send_eos();
            self.drain_frames();
        }
    }

    /// ワーカースレッドのエントリポイント。`job_rx` からジョブを処理するループを回す
    ///
    /// `Job::Terminate` を受け取るかチャネルが破棄された (`Err(_)`) ときに、
    /// [`DecodeWorker::finish`] で残りを完了させて return する。
    fn run(state: Box<DecoderState>, handler: H, job_rx: Receiver<Job<H::UserData>>) {
        let mut worker = DecodeWorker {
            state,
            handler,
            pending_user_data: VecDeque::new(),
            terminated: false,
        };

        loop {
            match job_rx.recv() {
                Ok(Job::Decode { data, user_data }) => worker.handle_decode(&data, user_data),
                Ok(Job::Flush { done }) => worker.handle_flush(done),
                Ok(Job::Terminate) | Err(_) => {
                    // チャネル破棄 (Err) 時も、残っている非同期処理を完了させてから終了する。
                    worker.finish();
                    return;
                }
            }
        }
    }

    /// キューに残った Ok フレームをすべて破棄する
    ///
    /// 失敗した parse 内で既に `frame_tx` に乗ったフレームを、
    /// 先行 pending と誤ペアリングしないために使う。
    fn discard_queued_frames(&self) {
        while self.state.next_frame().is_some() {}
    }

    /// 出力可能なフレームをすべて取り出してコールバックする
    ///
    /// `false` を返すのは、デコード結果に対応するユーザーデータが存在しない
    /// (missing user data) 場合で、通常あり得ない不変条件違反を意味する。
    /// そのときは継続せず終端にするため、呼び出し側で終端遷移を行う。
    fn drain_frames(&mut self) -> bool {
        loop {
            match self.state.next_frame() {
                None => {
                    // 結果が存在しなくなったなら終了
                    break;
                }
                Some(raw) => {
                    if let Some(user_data) = self.pending_user_data.pop_front() {
                        self.handler.on_decoded(Ok(DecodedFrame {
                            width: raw.width,
                            height: raw.height,
                            pitch: raw.pitch,
                            data: raw.data,
                            user_data,
                        }));
                    } else {
                        // デコード結果が存在するのに対応するユーザーデータが存在しない。
                        // 通常あり得ない不変条件違反なので、継続せず終端にする。
                        self.handler.on_decoded(Err(Error::new_custom(
                            "drain_frames",
                            "missing user data",
                        )
                        .into()));
                        return false;
                    }
                }
            }
        }

        true
    }
}

/// コールバックの失敗を `callback_error` に格納する。
/// 最初の 1 件だけ保持し、後続の失敗は破棄する。
///
/// 格納したエラーは [`DecoderState::prefer_callback_error`] 経由で利用者へ返す。
///
/// スレッド競合は起きない。パーサーコールバックは nvcuvid.h の保証により
/// `cuvidParseVideoData` 内で呼び出し元と同じスレッドから同期的に呼ばれ、
/// かつ `cuvidParseVideoData` はワーカースレッドの `decode` / `send_eos` からのみ呼ばれる。
/// そのため `callback_error` の読み書きはすべてワーカースレッド上で完結し、Mutex は不要である。
fn store_callback_error(state: &mut DecoderState, e: Error) {
    if state.callback_error.is_none() {
        state.callback_error = Some(e);
    }
}

/// 終端後のジョブに返すエラー
fn terminal_decode_error() -> Error {
    Error::new_custom("decode", "decoder has already failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// テスト用のデコーダー設定を生成する
    fn test_decoder_config(codec: DecoderCodec) -> DecoderConfig {
        DecoderConfig {
            codec,
            device_id: 0,
            max_num_decode_surfaces: 20,
            max_display_delay: 0,
            surface_format: SurfaceFormat::Nv12,
        }
    }

    /// 実効サーフェス数の決定規則の境界値を検証する (GPU 不要)
    #[test]
    fn determine_num_decode_surfaces_boundaries() {
        // 最小値も上限も 1 の場合は 1 を使う
        assert_eq!(
            DecoderState::determine_num_decode_surfaces(1, 1).expect("1 and 1 must be valid"),
            1
        );
        // 最小値が 1 で上限が 2 以上の場合は、parser の DPB 数を更新できる最小値 2 を使う
        assert_eq!(
            DecoderState::determine_num_decode_surfaces(1, 2).expect("1 and 2 must be valid"),
            2
        );
        assert_eq!(
            DecoderState::determine_num_decode_surfaces(1, 20).expect("1 and 20 must be valid"),
            2
        );
        // 最小値が 2 以上の場合はその値を使う
        assert_eq!(
            DecoderState::determine_num_decode_surfaces(2, 2).expect("2 and 2 must be valid"),
            2
        );
        assert_eq!(
            DecoderState::determine_num_decode_surfaces(8, 20).expect("8 and 20 must be valid"),
            8
        );
        // 最小値が上限を超える場合はエラーにする (max == 0 の場合も min > max で弾かれる)
        assert!(DecoderState::determine_num_decode_surfaces(9, 8).is_err());
        assert!(DecoderState::determine_num_decode_surfaces(1, 0).is_err());
        // 最小値が 0 の場合は NVDEC からの不正な値としてエラーにする
        assert!(DecoderState::determine_num_decode_surfaces(0, 20).is_err());
        assert!(DecoderState::determine_num_decode_surfaces(0, 0).is_err());
    }

    /// `max_num_decode_surfaces == 0` を `Decoder::new` が設定エラーとして拒否することを検証する
    ///
    /// 検証は CUDA ライブラリのロードより前に行われるため、GPU 不要で確認できる。
    #[test]
    fn decoder_rejects_zero_max_num_decode_surfaces() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let mut config = test_decoder_config(DecoderCodec::H264);
        config.max_num_decode_surfaces = 0;
        let error = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect_err("max_num_decode_surfaces == 0 must be rejected");
        assert!(
            error
                .to_string()
                .contains("max_num_decode_surfaces must be greater than 0")
        );
    }

    /// デコードされた黒フレームの検証を行う
    fn assert_black_frame(frame: &DecodedFrame<()>, expected_width: usize, expected_height: usize) {
        assert_eq!(frame.width(), expected_width);
        assert_eq!(frame.height(), expected_height);

        assert_eq!(frame.y_plane().len(), frame.y_stride() * frame.height());
        assert_eq!(
            frame.uv_plane().len(),
            frame.uv_stride() * frame.height().div_ceil(2)
        );

        assert!(frame.y_stride() >= frame.width());
        assert!(frame.uv_stride() >= frame.width());

        let y_data = frame.y_plane();
        let uv_data = frame.uv_plane();

        let y_avg = y_data.iter().map(|&x| x as u32).sum::<u32>() / y_data.len() as u32;
        assert!(
            (10..=30).contains(&y_avg),
            "Y average should be around 16 for black, got {}",
            y_avg
        );

        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            (70..=140).contains(&uv_avg),
            "UV average should be in reasonable range, got {}",
            uv_avg
        );
    }

    #[test]
    fn init_h264_decoder() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::H264);
        let _decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to initialize h264 decoder");
        println!("h264 decoder initialized successfully");
    }

    #[test]
    fn init_h265_decoder() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Hevc);
        let _decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to initialize h265 decoder");
        println!("h265 decoder initialized successfully");
    }

    #[test]
    fn init_av1_decoder() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Av1);
        let _decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to initialize av1 decoder");
        println!("av1 decoder initialized successfully");
    }

    #[test]
    fn init_vp8_decoder() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Vp8);
        let _decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to initialize vp8 decoder");
        println!("vp8 decoder initialized successfully");
    }

    #[test]
    fn init_vp9_decoder() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Vp9);
        let _decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to initialize vp9 decoder");
        println!("vp9 decoder initialized successfully");
    }

    #[test]
    fn test_multiple_decoders() {
        let config = test_decoder_config(DecoderCodec::Hevc);
        let (tx1, _rx1) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let _decoder1 = Decoder::new(
            config.clone(),
            FnDecodeHandler::new(move |frame| {
                let _ = tx1.send(frame);
            }),
        )
        .expect("Failed to initialize first h265 decoder");

        let (tx2, _rx2) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let _decoder2 = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx2.send(frame);
            }),
        )
        .expect("Failed to initialize second h265 decoder");
        println!("Multiple h265 decoders initialized successfully");
    }

    #[test]
    fn test_decode_h265_black_frame() {
        // H.265 の黒フレームデータ (Annex B format with start codes)
        // VPS, SPS, PPS, Frame data を Annex B 形式で結合
        let vps = vec![
            64, 1, 12, 1, 255, 255, 1, 96, 0, 0, 3, 0, 144, 0, 0, 3, 0, 0, 3, 0, 90, 149, 152, 9,
        ];
        let sps = vec![
            66, 1, 1, 1, 96, 0, 0, 3, 0, 144, 0, 0, 3, 0, 0, 3, 0, 90, 160, 5, 2, 1, 225, 101, 149,
            154, 73, 50, 188, 5, 160, 32, 0, 0, 3, 0, 32, 0, 0, 3, 3, 33,
        ];
        let pps = vec![68, 1, 193, 114, 180, 98, 64];
        let frame_data = vec![
            40, 1, 175, 29, 16, 90, 181, 140, 90, 213, 247, 1, 91, 255, 242, 78, 254, 199, 0, 31,
            209, 50, 148, 21, 162, 38, 146, 0, 0, 3, 1, 203, 169, 113, 202, 5, 24, 129, 39, 128, 0,
            0, 3, 0, 7, 204, 147, 13, 148, 32, 0, 0, 3, 0, 0, 3, 0, 12, 24, 135, 0, 0, 3, 0, 0, 3,
            0, 0, 3, 0, 28, 240, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 8, 104, 0, 0, 3, 0, 0, 3, 0, 0, 3,
            0, 104, 192, 0, 0, 3, 0, 0, 3, 0, 0, 3, 1, 223, 0, 0, 3, 0, 9, 248,
        ];

        // NAL ユニットを結合（Annex B 形式: start code 0x00000001 を使用）
        let mut h265_data = Vec::new();
        let start_code = [0u8, 0, 0, 1];

        // VPS
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&vps);

        // SPS
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&sps);

        // PPS
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&pps);

        // Frame data
        h265_data.extend_from_slice(&start_code);
        h265_data.extend_from_slice(&frame_data);

        let config = test_decoder_config(DecoderCodec::Hevc);
        let (tx, rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to create h265 decoder");

        // デコードを実行
        decoder
            .decode(&h265_data, ())
            .expect("Failed to decode H.265 data");

        // フィニッシュ処理をテスト
        decoder.flush().expect("flush failed");

        // デコード済みフレームを取得
        let frame = rx
            .recv()
            .expect("No decoded frame available")
            .expect("Decoding error occurred");

        assert_black_frame(&frame, 640, 480);

        drop(decoder);
    }

    #[test]
    fn test_decode_h264_black_frame() {
        // H.264 の黒フレームデータ (NAL units with size prefix)
        let sps = vec![
            103, 100, 0, 30, 172, 217, 64, 160, 61, 176, 17, 0, 0, 3, 0, 1, 0, 0, 3, 0, 50, 15, 22,
            45, 150,
        ];
        let pps = vec![104, 235, 227, 203, 34, 192];
        let frame_data = vec![
            101, 136, 132, 0, 43, 255, 254, 246, 115, 124, 10, 107, 109, 176, 149, 46, 5, 118, 247,
            102, 163, 229, 208, 146, 229, 251, 16, 96, 250, 208, 0, 0, 3, 0, 0, 3, 0, 0, 16, 15,
            210, 222, 245, 204, 98, 91, 229, 32, 0, 0, 9, 216, 2, 56, 13, 16, 118, 133, 116, 69,
            196, 32, 71, 6, 120, 150, 16, 161, 210, 50, 128, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0,
            0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 37, 225,
        ];

        // NAL ユニットを結合（Annex B 形式: start code 0x00000001 を使用）
        let mut h264_data = Vec::new();
        let start_code = [0u8, 0, 0, 1];

        // SPS
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&sps);

        // PPS
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&pps);

        // Frame data
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&frame_data);

        let config = test_decoder_config(DecoderCodec::H264);
        let (tx, rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to create h264 decoder");

        // デコードを実行
        decoder
            .decode(&h264_data, ())
            .expect("Failed to decode H.264 data");

        // フィニッシュ処理をテスト
        decoder.flush().expect("flush failed");

        // デコード済みフレームを取得
        let frame = rx
            .recv()
            .expect("No decoded frame available")
            .expect("Decoding error occurred");

        assert_black_frame(&frame, 640, 480);

        drop(decoder);
    }

    #[test]
    fn test_decode_av1_black_frame() {
        // AV1 の黒フレームデータ (OBU format)
        // OBU_TYPE=1 (sequence header) と OBU_TYPE=6 (frame) を含む
        let av1_data = vec![
            // TYPE=1 (Sequence Header OBU)
            10, 11, 0, 0, 0, 36, 196, 255, 223, 63, 254, 96, 16, // TYPE=6 (Frame OBU)
            50, 35, 16, 0, 144, 0, 0, 0, 160, 0, 0, 128, 1, 197, 120, 80, 103, 179, 239, 241, 100,
            76, 173, 116, 93, 183, 31, 101, 221, 87, 90, 233, 219, 28, 199, 243, 128,
        ];

        let config = test_decoder_config(DecoderCodec::Av1);
        let (tx, rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to create av1 decoder");

        // デコードを実行
        decoder
            .decode(&av1_data, ())
            .expect("Failed to decode AV1 data");

        // フィニッシュ処理をテスト
        decoder.flush().expect("flush failed");

        // デコード済みフレームを取得
        let frame = rx
            .recv()
            .expect("No decoded frame available")
            .expect("Decoding error occurred");

        assert_black_frame(&frame, 640, 480);

        drop(decoder);
    }

    #[test]
    fn test_decode_vp8_black_frame() {
        // VP8 の黒フレームデータ
        let vp8_data = vec![
            80, 66, 0, 157, 1, 42, 128, 2, 224, 1, 2, 199, 8, 133, 133, 136, 153, 132, 136, 15, 2,
            0, 6, 22, 4, 247, 6, 129, 100, 159, 107, 219, 155, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 55, 128, 254,
            250, 215, 128,
        ];

        let config = test_decoder_config(DecoderCodec::Vp8);
        let (tx, rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to create vp8 decoder");

        // デコードを実行
        decoder
            .decode(&vp8_data, ())
            .expect("Failed to decode VP8 data");

        // フィニッシュ処理をテスト
        decoder.flush().expect("flush failed");

        // デコード済みフレームを取得
        let frame = rx
            .recv()
            .expect("No decoded frame available")
            .expect("Decoding error occurred");

        assert_black_frame(&frame, 640, 480);

        drop(decoder);
    }

    #[test]
    fn test_decode_vp9_black_frame() {
        // VP9 の黒フレームデータ
        let vp9_data = vec![
            130, 73, 131, 66, 0, 39, 240, 29, 246, 0, 56, 36, 28, 24, 74, 16, 0, 80, 97, 246, 58,
            246, 128, 92, 209, 238, 0, 0, 0, 0, 0, 20, 103, 26, 154, 224, 98, 35, 126, 68, 120,
            240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0, 103, 26, 154, 224, 98, 35, 126, 68,
            120, 240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0,
        ];

        let config = test_decoder_config(DecoderCodec::Vp9);
        let (tx, rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to create vp9 decoder");

        // デコードを実行
        decoder
            .decode(&vp9_data, ())
            .expect("Failed to decode VP9 data");

        // フィニッシュ処理をテスト
        decoder.flush().expect("flush failed");

        // デコード済みフレームを取得
        let frame = rx
            .recv()
            .expect("No decoded frame available")
            .expect("Decoding error occurred");

        assert_black_frame(&frame, 640, 480);

        drop(decoder);
    }

    #[test]
    fn test_decode_after_worker_terminated() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::H264);

        let mut decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("H.264 デコーダーの作成に失敗した");

        // 受信側を先に drop したチャネルで置き換える。
        // この代入で元の job_tx が drop され、ワーカースレッドは
        // recv の Err を検知して終了する。
        let (dead_tx, dead_rx) = mpsc::sync_channel::<Job<()>>(4);
        drop(dead_rx);
        decoder.job_tx = dead_tx;

        let result = decoder.decode(&[], ());
        assert_eq!(
            result.unwrap_err().to_string(),
            "decode() failed: decoder worker thread has terminated"
        );
    }

    #[test]
    fn test_flush_after_decoder_worker_terminated() {
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::H264);

        let mut decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("H.264 デコーダーの作成に失敗した");

        // 受信側を先に drop したチャネルで置き換える。
        // この代入で元の job_tx が drop され、ワーカースレッドは
        // recv の Err を検知して終了する。
        let (dead_tx, dead_rx) = mpsc::sync_channel::<Job<()>>(4);
        drop(dead_rx);
        decoder.job_tx = dead_tx;

        let result = decoder.flush();
        assert_eq!(
            result.unwrap_err().to_string(),
            "flush() failed: send failed"
        );
    }

    #[test]
    fn test_query_decoder_caps_h264() {
        let caps = query_decoder_caps(DecoderCodec::H264, 0)
            .expect("query_decoder_caps for H264 should succeed");
        assert!(
            caps.max_width > 0,
            "max_width should be positive: {}",
            caps.max_width
        );
        assert!(
            caps.max_height > 0,
            "max_height should be positive: {}",
            caps.max_height
        );
    }

    /// H.264 の黒フレームデータ (640x480) を生成する
    ///
    /// test_decode_h264_black_frame と同じ SPS / PPS / フレームデータを
    /// Annex B 形式 (start code 0x00000001) で結合する。
    fn h264_black_frame_data() -> Vec<u8> {
        let sps = vec![
            103, 100, 0, 30, 172, 217, 64, 160, 61, 176, 17, 0, 0, 3, 0, 1, 0, 0, 3, 0, 50, 15, 22,
            45, 150,
        ];
        let pps = vec![104, 235, 227, 203, 34, 192];
        let frame_data = vec![
            101, 136, 132, 0, 43, 255, 254, 246, 115, 124, 10, 107, 109, 176, 149, 46, 5, 118, 247,
            102, 163, 229, 208, 146, 229, 251, 16, 96, 250, 208, 0, 0, 3, 0, 0, 3, 0, 0, 16, 15,
            210, 222, 245, 204, 98, 91, 229, 32, 0, 0, 9, 216, 2, 56, 13, 16, 118, 133, 116, 69,
            196, 32, 71, 6, 120, 150, 16, 161, 210, 50, 128, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0,
            0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 37, 225,
        ];

        let mut h264_data = Vec::new();
        let start_code = [0u8, 0, 0, 1];

        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&sps);
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&pps);
        h264_data.extend_from_slice(&start_code);
        h264_data.extend_from_slice(&frame_data);

        h264_data
    }

    /// stats() でデコーダーの統計値が取得できることを確認する
    ///
    /// H.264 の 1 フレームをデコードすると:
    /// - total_create_decoder_count が 1 (シーケンスコールバックで cuvidCreateDecoder が 1 回呼ばれる)
    /// - total_decode_count が 1 (decode() を 1 回呼んだ)
    /// - total_sequence_callback_count が 1 (シーケンスコールバックが 1 回呼ばれる)
    /// - total_decode_callback_count が 1 (デコードコールバックが 1 回呼ばれる)
    /// - total_output_frame_count が 1 (1 フレームが出力される)
    #[test]
    fn test_decoder_stats_counters() {
        let config = test_decoder_config(DecoderCodec::H264);
        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("H.264 デコーダーの作成に失敗した");

        // デコード前はすべて 0 である
        assert_eq!(decoder.stats().total_create_decoder_count.get(), 0);
        assert_eq!(decoder.stats().total_decode_count.get(), 0);
        assert_eq!(decoder.stats().total_sequence_callback_count.get(), 0);
        assert_eq!(decoder.stats().total_decode_callback_count.get(), 0);
        assert_eq!(decoder.stats().total_output_frame_count.get(), 0);

        // デコードを実行
        decoder
            .decode(&h264_black_frame_data(), ())
            .expect("H.264 データのデコードに失敗した");
        decoder.flush().expect("flush に失敗した");

        // シーケンスコールバックで cuvidCreateDecoder が 1 回呼ばれる
        assert_eq!(decoder.stats().total_create_decoder_count.get(), 1);
        // decode() を 1 回呼んだ
        assert_eq!(decoder.stats().total_decode_count.get(), 1);
        // シーケンスコールバックが 1 回呼ばれる
        assert_eq!(decoder.stats().total_sequence_callback_count.get(), 1);
        // デコードコールバックが 1 回呼ばれる
        assert_eq!(decoder.stats().total_decode_callback_count.get(), 1);
        // 1 フレームが出力される
        assert_eq!(decoder.stats().total_output_frame_count.get(), 1);
        // 1 バッファに 1 フレームを渡しているため、flush 後の in-flight (入力 - 出力) は 0 になる
        assert_eq!(decoder.stats().in_flight_frames(), 0);

        drop(decoder);
    }

    /// Annex-B ストリームをアクセスユニット (フレーム) 単位に分割する
    ///
    /// NAL ユニットの開始コード (00 00 01 / 00 00 00 01) を検出し、
    /// VCL NAL (スライス) の直前で区切ることで 1 フレーム = 1 アクセスユニットにする
    /// (テスト用ストリームは 1 フレーム = 1 スライスのため)
    fn split_annexb_frames(data: &[u8], is_vcl: fn(u8) -> bool) -> Vec<&[u8]> {
        // NAL ユニットの開始位置を列挙する
        // 4 バイト開始コード (00 00 00 01) は 3 バイト目も 3 バイト開始コード (00 00 01) に
        // マッチするため、直前のバイトが 0 なら 4 バイト開始コードとして扱う
        let mut nal_starts = Vec::new();
        let mut i = 0;
        while i + 3 < data.len() {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                if i > 0 && data[i - 1] == 0 {
                    nal_starts.push(i - 1);
                } else {
                    nal_starts.push(i);
                }
                i += 3;
            } else {
                i += 1;
            }
        }
        // VCL NAL の開始位置がアクセスユニットの境界になる
        let vcl_starts: Vec<usize> = nal_starts
            .iter()
            .copied()
            .filter(|&s| {
                let header =
                    if s + 4 < data.len() && data[s] == 0 && data[s + 1] == 0 && data[s + 2] == 0 {
                        data[s + 4]
                    } else {
                        data[s + 3]
                    };
                is_vcl(header)
            })
            .collect();
        let mut frames = Vec::new();
        for (i, &start) in vcl_starts.iter().enumerate() {
            // 先頭フレームには先行する非 VCL NAL (SEI / SPS / PPS / VPS など) が
            // 含まれるため、ストリーム先頭から開始する
            let start = if i == 0 { 0 } else { start };
            let end = vcl_starts.get(i + 1).copied().unwrap_or(data.len());
            frames.push(&data[start..end]);
        }
        frames
    }

    /// IVF コンテナをフレーム単位に分割する
    ///
    /// IVF のフレームヘッダ (サイズ 4 バイト + タイムスタンプ 8 バイト) を
    /// 辿ってペイロード (1 フレーム = 1 ピクチャ) を取り出す
    fn split_ivf_frames(data: &[u8]) -> Vec<&[u8]> {
        // ヘッダ (32 バイト) を読み飛ばす
        let mut offset = 32;
        let mut frames = Vec::new();
        while offset + 12 <= data.len() {
            let size = u32::from_le_bytes(
                data[offset..offset + 4]
                    .try_into()
                    .expect("決して失敗しないはず"),
            ) as usize;
            offset += 12;
            frames.push(&data[offset..offset + size]);
            offset += size;
        }
        frames
    }

    /// テストデータを 1 フレームずつデコードして、フレームとエラーを収集する
    ///
    /// 戻り値は (デコードされたフレーム, エラー, total_create_decoder_count)。
    /// 通常の decoder 再作成 (destroy + create) 経路の検証に使う。
    fn decode_resolution_change_data(
        codec: DecoderCodec,
        frames: &[&[u8]],
    ) -> (Vec<DecodedFrame<()>>, Vec<Error>, u64) {
        let config = test_decoder_config(codec);
        let (tx, rx) = mpsc::channel();
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("デコーダーの作成に失敗した");

        for frame in frames {
            // シーケンスコールバックのエラーは decode の戻り値にも伝播するが、
            // 具体的な内容はハンドラ経由で通知されるため戻り値は確認しない
            let _ = decoder.decode(frame, ());
        }
        let _ = decoder.flush();

        // destroy + create 経路ではシーケンス変更ごとに cuvidCreateDecoder が呼ばれる
        let create_count = decoder.stats().total_create_decoder_count.get();

        // チャネルからフレームとエラーを回収する
        let mut decoded_frames = Vec::new();
        let mut errors = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(Ok(frame)) => decoded_frames.push(frame),
                Ok(Err(e)) => errors.push(e),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }
        (decoded_frames, errors, create_count)
    }

    /// 通常の decoder 再作成 (destroy + create) 経路で 45 フレームすべてがデコードされることを確認する
    ///
    /// 320x240 x30 + 256x160 x15 の解像度変化ストリームを、シーケンス変更ごとの
    /// decoder 再作成で欠落なくデコードできることを確認する。display_delay = 0 のため
    /// シーケンス変更時に in-flight フレームが存在せず、フレームロスは発生しないことを期待する。
    fn assert_resolution_change_frames_destroy_and_recreate(codec: DecoderCodec, frames: &[&[u8]]) {
        let (decoded_frames, errors, create_count) = decode_resolution_change_data(codec, frames);

        // エラーが 1 件も通知されないことを確認する
        assert!(
            errors.is_empty(),
            "予期しないエラーが通知された: {errors:?}"
        );

        // destroy + create 経路では、シーケンス変更ごとに cuvidCreateDecoder が呼ばれる
        assert!(
            create_count >= 2,
            "シーケンス変更ごとに cuvidCreateDecoder が呼ばれるはず (codec: {codec:?}): {create_count}"
        );

        // 全フレームがデコードされることを確認する
        assert_eq!(
            decoded_frames.len(),
            frames.len(),
            "全フレームがデコードされるはず (codec: {codec:?}): {}",
            decoded_frames.len()
        );

        // 各フレームの寸法を検証する
        let mut size_counts = std::collections::HashMap::new();
        for frame in &decoded_frames {
            size_counts
                .entry((frame.width(), frame.height()))
                .and_modify(|c| *c += 1)
                .or_insert(1);
        }
        assert_eq!(size_counts.get(&(320, 240)), Some(&30), "codec: {codec:?}");
        assert_eq!(size_counts.get(&(256, 160)), Some(&15), "codec: {codec:?}");
        assert_eq!(size_counts.len(), 2, "codec: {codec:?}");
    }

    #[test]
    fn test_split_annexb_frames_h264() {
        // H.264 テストデータは 45 アクセスユニットに分割される
        let data = include_bytes!("../testdata/resolution-change/h264.h264");
        let frames = split_annexb_frames(data, |nal| (nal & 0x1f) == 1 || (nal & 0x1f) == 5);
        assert_eq!(frames.len(), 45, "h264 フレーム数");

        // 先頭フレームには SPS (NAL type 7) が含まれる
        assert!(
            frames[0]
                .windows(4)
                .any(|w| w[..3] == [0, 0, 1] && (w[3] & 0x1f) == 7),
            "先頭フレームに SPS が含まれるはず"
        );
    }

    #[test]
    fn test_split_annexb_frames_h265() {
        // H.265 テストデータは 45 アクセスユニットに分割される
        let data = include_bytes!("../testdata/resolution-change/h265.h265");
        let frames = split_annexb_frames(data, |nal| nal >> 1 <= 31);
        assert_eq!(frames.len(), 45, "h265 フレーム数");

        // 先頭フレームには VPS (NAL type 32) が含まれる
        assert!(
            frames[0].windows(4).any(|w| w == [0, 0, 1, 0x40]),
            "先頭フレームに VPS が含まれるはず"
        );
    }

    #[test]
    fn test_split_ivf_frames() {
        // IVF テストデータは 45 フレームに分割される
        let vp8_data = include_bytes!("../testdata/resolution-change/vp8.ivf");
        assert_eq!(split_ivf_frames(vp8_data).len(), 45, "vp8 フレーム数");
        let vp9_data = include_bytes!("../testdata/resolution-change/vp9.ivf");
        assert_eq!(split_ivf_frames(vp9_data).len(), 45, "vp9 フレーム数");
        let av1_data = include_bytes!("../testdata/resolution-change/av1.ivf");
        assert_eq!(split_ivf_frames(av1_data).len(), 45, "av1 フレーム数");
    }

    #[test]
    fn test_decode_h264_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合 (通常経路) は
        // destroy + create で解像度変化に対応することを確認する
        let data = include_bytes!("../testdata/resolution-change/h264.h264");
        let frames = split_annexb_frames(data, |nal| (nal & 0x1f) == 1 || (nal & 0x1f) == 5);
        assert_eq!(frames.len(), 45, "h264 フレーム数");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::H264, &frames);
    }

    #[test]
    fn test_decode_h265_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合 (通常経路) は
        // destroy + create で解像度変化に対応することを確認する
        let data = include_bytes!("../testdata/resolution-change/h265.h265");
        let frames = split_annexb_frames(data, |nal| nal >> 1 <= 31);
        assert_eq!(frames.len(), 45, "h265 フレーム数");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Hevc, &frames);
    }

    #[test]
    fn test_decode_vp8_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合 (通常経路) は
        // destroy + create で解像度変化に対応することを確認する
        let data = include_bytes!("../testdata/resolution-change/vp8.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "vp8 フレーム数");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Vp8, &frames);
    }

    #[test]
    fn test_decode_vp9_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合 (通常経路) は
        // destroy + create で解像度変化に対応することを確認する
        let data = include_bytes!("../testdata/resolution-change/vp9.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "vp9 フレーム数");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Vp9, &frames);
    }

    #[test]
    fn test_decode_av1_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合 (通常経路) は
        // destroy + create で解像度変化に対応することを確認する
        let data = include_bytes!("../testdata/resolution-change/av1.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "av1 フレーム数");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Av1, &frames);
    }
}
