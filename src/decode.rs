use std::collections::VecDeque;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

use crate::{CudaLibrary, Error, sys};

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

    /// デコード用サーフェスの最大数
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
    width: u32,
    height: u32,
    surface_width: u32,
    surface_height: u32,
    surface_format: u32,
    frame_tx: Sender<Result<RawFrame, Error>>,
    frame_rx: Receiver<Result<RawFrame, Error>>,
}

unsafe impl Send for DecoderState {}

impl DecoderState {
    /// 指定されたコーデック設定でデコーダーインスタンスを生成する
    pub fn new(config: DecoderConfig) -> Result<Box<Self>, Error> {
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
    pub fn query_caps(codec: DecoderCodec, device_id: i32) -> Result<DecoderCaps, Error> {
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
                width: 0,
                height: 0,
                surface_width: 0,
                surface_height: 0,
                surface_format: config.surface_format.to_sys(),
                frame_tx,
                frame_rx,
            });

            // 映像パーサーを作成する
            let mut parser_params: sys::CUVIDPARSERPARAMS = std::mem::zeroed();
            parser_params.CodecType = codec_type;
            parser_params.ulMaxNumDecodeSurfaces = config.max_num_decode_surfaces;
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

    /// 圧縮された映像フレームをデコードする
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
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

            self.lib.cuvid_parse_video_data(self.parser, &mut packet)?;
        }

        Ok(())
    }

    pub fn send_eos(&mut self) -> Result<(), Error> {
        unsafe {
            // EOS をデコーダーに伝える
            let mut packet: sys::CUVIDSOURCEDATAPACKET = std::mem::zeroed();
            packet.payload = ptr::null();
            packet.payload_size = 0;
            packet.flags = sys::CUvideopacketflags_CUVID_PKT_ENDOFSTREAM as u64;
            packet.timestamp = 0;

            self.lib.cuvid_parse_video_data(self.parser, &mut packet)?;

            // パーサーは非同期でデータを処理するので、
            // すべてのデコード操作が完了するまでここで待機（同期）する
            self.lib
                .with_context(self.ctx, || self.lib.cu_ctx_synchronize())?;
        }
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    pub fn next_frame(&mut self) -> Result<Option<RawFrame>, Error> {
        self.frame_rx.try_recv().ok().transpose()
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

/// デコーダー
///
/// 内部で専用のワーカースレッドを起動し、非同期でデコードを行う。
/// デコードが完了すると、コンストラクタで渡したコールバックがワーカースレッド上で即座に呼び出される。
pub struct Decoder<T> {
    job_tx: SyncSender<Job<T>>,
    worker: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> Decoder<T> {
    /// デコーダーを生成し、内部ワーカースレッドを起動する
    pub fn new<F>(config: DecoderConfig, mut callback: F) -> Result<Self, Error>
    where
        F: FnMut(Result<DecodedFrame<T>, Error>) + Send + 'static,
    {
        let (job_tx, job_rx) = mpsc::sync_channel::<Job<T>>(4);

        let state = DecoderState::new(config)?;

        let worker = std::thread::Builder::new()
            .name("nvcodec-decoder".into())
            .spawn(move || {
                run_worker(state, &mut callback, job_rx);
            })
            .map_err(|_e| Error::new_custom("Decoder::new", "failed to spawn decoder thread"))?;

        Ok(Self {
            job_tx,
            worker: Some(worker),
        })
    }

    /// 圧縮された映像フレームをデコードする
    ///
    /// フレームデータとユーザーデータをワーカースレッドに送信し、即座に戻る。
    /// デコードが完了すると、コンストラクタで渡したコールバックが呼び出される。
    pub fn decode(&self, data: &[u8], user_data: T) -> Result<(), Error> {
        self.job_tx
            .send(Job::Decode {
                data: data.to_vec(),
                user_data,
            })
            .map_err(|_| Error::new_custom("decode", "decoder worker thread has terminated"))
    }

    /// 送信済みの未完了フレームがすべて完了するまで待機する
    ///
    /// すべての pending フレームのコールバックが呼び出された後、このメソッドが戻る。
    /// flush 後も decode を継続できる。
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

impl<T> Drop for Decoder<T> {
    fn drop(&mut self) {
        let _ = self.job_tx.send(Job::Terminate);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl<T> std::fmt::Debug for Decoder<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder").finish_non_exhaustive()
    }
}

unsafe impl<T: Send> Send for Decoder<T> {}

/// 指定コーデックのデコーダのケーパビリティをクエリする
pub fn query_decoder_caps(codec: DecoderCodec, device_id: i32) -> Result<DecoderCaps, Error> {
    DecoderState::query_caps(codec, device_id)
}

// パーサーがシーケンスヘッダーを検出した時に呼ばれるコールバック
unsafe extern "C" fn handle_video_sequence(
    user_data: *mut c_void,
    format: *mut sys::CUVIDEOFORMAT,
) -> i32 {
    if user_data.is_null() || format.is_null() {
        return 0;
    }

    let state = unsafe { &mut *(user_data as *mut DecoderState) };

    // FFI コールバック内の panic はプロセス abort に直結するため catch_unwind で隔離する
    let result = catch_unwind(AssertUnwindSafe(|| {
        let format = unsafe { &*format };

        let result = handle_video_sequence_inner(state, format);
        match result {
            Ok(num_surfaces) => num_surfaces,
            Err(e) => {
                let _ = state.frame_tx.send(Err(e));
                0
            }
        }
    }));

    match result {
        Ok(v) => v,
        Err(_) => {
            // panic を検知したことを利用側に伝える
            let _ = state.frame_tx.send(Err(Error::new_custom(
                "handle_video_sequence",
                "panic occurred in FFI callback",
            )));
            0
        }
    }
}

fn handle_video_sequence_inner(
    state: &mut DecoderState,
    format: &sys::CUVIDEOFORMAT,
) -> Result<i32, Error> {
    // デコーダーが既に作成されている場合は破棄して再作成する
    // ストリーム中の解像度変更に対応するため
    if !state.decoder.is_null() {
        state
            .lib
            .with_context(state.ctx, || state.lib.cuvid_destroy_decoder(state.decoder))?;
        state.decoder = ptr::null_mut();
    }

    // デコーダーの作成情報を設定
    let mut create_info: sys::CUVIDDECODECREATEINFO = unsafe { std::mem::zeroed() };
    create_info.CodecType = format.codec;
    create_info.ChromaFormat = format.chroma_format;
    create_info.OutputFormat = state.surface_format;
    create_info.bitDepthMinus8 = format.bit_depth_luma_minus8 as u64;
    create_info.DeinterlaceMode = if format.progressive_sequence != 0 {
        sys::cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Weave
    } else {
        sys::cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Adaptive
    };
    create_info.ulNumOutputSurfaces = 2; // 出力サーフェスの数（ダブルバッファリング用に2を指定）
    create_info.ulCreationFlags = sys::cudaVideoCreateFlags_enum_cudaVideoCreate_PreferCUVID as u64; // CUVID ハードウェアデコーダーの使用を優先するフラグ
    create_info.ulNumDecodeSurfaces = format.min_num_decode_surfaces as u64;
    create_info.ulWidth = format.coded_width as u64;
    create_info.ulHeight = format.coded_height as u64;
    create_info.ulMaxWidth = format.coded_width as u64;
    create_info.ulMaxHeight = format.coded_height as u64;
    create_info.ulTargetWidth = format.coded_width as u64;
    create_info.ulTargetHeight = format.coded_height as u64;

    // パーサーと共有するコンテキストロックを使用
    create_info.vidLock = state.ctx_lock;

    state.lib.with_context(state.ctx, || {
        state
            .lib
            .cuvid_create_decoder(&mut state.decoder, &mut create_info)
    })?;
    // display_area は signed 整数のため、壊れたストリームで負値になる可能性がある
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
    state.width = (right - left) as u32;
    state.height = (bottom - top) as u32;
    state.surface_width = format.coded_width;
    state.surface_height = format.coded_height;

    Ok(format.min_num_decode_surfaces as i32)
}

// デコードすべきピクチャーがある時に呼ばれるコールバック
unsafe extern "C" fn handle_picture_decode(
    user_data: *mut c_void,
    pic_params: *mut sys::CUVIDPICPARAMS,
) -> i32 {
    if user_data.is_null() || pic_params.is_null() {
        return 0;
    }

    let state = unsafe { &mut *(user_data as *mut DecoderState) };

    // FFI コールバック内の panic はプロセス abort に直結するため catch_unwind で隔離する
    let result = catch_unwind(AssertUnwindSafe(|| {
        let result = handle_picture_decode_inner(state, unsafe { &*pic_params });
        match result {
            Ok(_) => 1,
            Err(e) => {
                let _ = state.frame_tx.send(Err(e));
                0
            }
        }
    }));

    match result {
        Ok(v) => v,
        Err(_) => {
            // panic を検知したことを利用側に伝える
            let _ = state.frame_tx.send(Err(Error::new_custom(
                "handle_picture_decode",
                "panic occurred in FFI callback",
            )));
            0
        }
    }
}

fn handle_picture_decode_inner(
    state: &mut DecoderState,
    pic_params: &sys::CUVIDPICPARAMS,
) -> Result<(), Error> {
    if state.decoder.is_null() {
        return Err(Error::new_custom(
            "handle_picture_decode",
            "decoder not initialized",
        ));
    }

    state.lib.with_context(state.ctx, || {
        state
            .lib
            .cuvid_decode_picture(state.decoder, pic_params as *const _ as *mut _)
    })?;

    Ok(())
}

// デコード済みフレームを表示する時に呼ばれるコールバック
unsafe extern "C" fn handle_picture_display(
    user_data: *mut c_void,
    disp_info: *mut sys::CUVIDPARSERDISPINFO,
) -> i32 {
    if user_data.is_null() || disp_info.is_null() {
        return 0;
    }

    let state = unsafe { &mut *(user_data as *mut DecoderState) };

    // FFI コールバック内の panic はプロセス abort に直結するため catch_unwind で隔離する
    let result = catch_unwind(AssertUnwindSafe(|| {
        let result = handle_picture_display_inner(state, unsafe { &*disp_info });
        match result {
            Ok(_) => 1,
            Err(e) => {
                let _ = state.frame_tx.send(Err(e));
                0
            }
        }
    }));

    match result {
        Ok(v) => v,
        Err(_) => {
            // panic を検知したことを利用側に伝える
            let _ = state.frame_tx.send(Err(Error::new_custom(
                "handle_picture_display",
                "panic occurred in FFI callback",
            )));
            0
        }
    }
}

fn handle_picture_display_inner(
    state: &DecoderState,
    disp_info: &sys::CUVIDPARSERDISPINFO,
) -> Result<(), Error> {
    if state.decoder.is_null() {
        return Err(Error::new_custom(
            "handle_picture_display",
            "decoder not initialized",
        ));
    }

    let decoded_frame = state.lib.with_context(state.ctx, || unsafe {
        // ビデオ処理パラメーターを設定
        let mut proc_params: sys::CUVIDPROCPARAMS = std::mem::zeroed();
        proc_params.progressive_frame = disp_info.progressive_frame;
        proc_params.top_field_first = disp_info.top_field_first;
        proc_params.second_field = disp_info.repeat_first_field + 1;
        proc_params.output_stream = ptr::null_mut();

        // デコード済みフレームをマップ
        let mut device_ptr = 0u64;
        let mut pitch = 0u32;
        state.lib.cuvid_map_video_frame(
            state.decoder,
            disp_info.picture_index,
            &mut device_ptr,
            &mut pitch,
            &mut proc_params,
        )?;

        // 確実にフレームをアンマップするためのガードを作成
        let _unmap_guard = crate::ReleaseGuard::new(|| {
            let _ = state.lib.cuvid_unmap_video_frame(state.decoder, device_ptr);
        });

        // フレームサイズを計算 (NV12 形式: Y プレーン + UV プレーン)
        // 注意: NVDEC は高さを 2 でアライメントする
        let aligned_height = (state.surface_height + 1) & !1;
        let y_size = pitch as usize * state.height as usize;
        let uv_size = pitch as usize * (state.height as usize).div_ceil(2);
        let frame_size = y_size + uv_size;

        // フレーム用のホストメモリを割り当て
        let mut host_data = vec![0u8; frame_size];

        // Y プレーンをコピー
        state
            .lib
            .cu_memcpy_d_to_h(host_data.as_mut_ptr() as *mut c_void, device_ptr, y_size)?;

        // UV プレーンをコピー
        let uv_offset = pitch as u64 * aligned_height as u64;
        state.lib.cu_memcpy_d_to_h(
            host_data[y_size..].as_mut_ptr() as *mut c_void,
            device_ptr + uv_offset,
            uv_size,
        )?;

        // デコード済みフレームを作成
        Ok(RawFrame {
            width: state.width,
            height: state.height,
            pitch: pitch as usize,
            data: host_data,
        })
    })?;

    // チャンネル経由で送信 (受信側が破棄されている場合の送信エラーは無視)
    let _ = state.frame_tx.send(Ok(decoded_frame));

    Ok(())
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
    pub fn width(&self) -> usize {
        self.width as usize
    }

    /// フレームの高さを返す
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

fn run_worker<F, T>(mut state: Box<DecoderState>, callback: &mut F, job_rx: Receiver<Job<T>>)
where
    F: FnMut(Result<DecodedFrame<T>, Error>) + Send + 'static,
    T: Send + 'static,
{
    let mut pending_user_data: VecDeque<T> = VecDeque::new();

    loop {
        match job_rx.recv() {
            Ok(Job::Decode { data, user_data }) => {
                if let Err(e) = state.decode(&data) {
                    callback(Err(e));
                    continue;
                }

                pending_user_data.push_back(user_data);
                drain_frames(&mut state, callback, &mut pending_user_data);
            }
            Ok(Job::Flush { done }) => {
                let _ = state.send_eos();

                drain_frames(&mut state, callback, &mut pending_user_data);

                let _ = done.send(());
            }
            Ok(Job::Terminate) | Err(_) => {
                // 残っている非同期処理を完了させる
                let _ = state.send_eos();

                drain_frames(&mut state, callback, &mut pending_user_data);

                // state の Drop がここで走り、CUDA リソースが解放される
                return;
            }
        }
    }
}

fn drain_frames<F, T>(
    state: &mut DecoderState,
    callback: &mut F,
    pending_user_data: &mut VecDeque<T>,
) where
    F: FnMut(Result<DecodedFrame<T>, Error>) + Send + 'static,
    T: Send + 'static,
{
    loop {
        match state.next_frame() {
            Ok(None) => {
                // 結果が存在しなくなったなら終了
                break;
            }
            Ok(Some(raw)) => {
                if let Some(user_data) = pending_user_data.pop_front() {
                    callback(Ok(DecodedFrame {
                        width: raw.width,
                        height: raw.height,
                        pitch: raw.pitch,
                        data: raw.data,
                        user_data,
                    }));
                } else {
                    // デコード結果が存在するのに対応するユーザーデータが存在しない
                    // これは通常あり得ないはずだけど、エラーを取りこぼさない為にエラーのコールバックを呼ぶ
                    callback(Err(Error::new_custom("drain_frames", "missing user data")));
                    break;
                }
            }
            // エラーが起きたら全てのユーザーデータを削除して
            // コールバックを呼ぶ
            Err(e) => {
                pending_user_data.clear();
                callback(Err(e));
                break;
            }
        }
    }
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

    #[test]
    fn init_h264_decoder() {
        let (_tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::H264);
        let _decoder = Decoder::new(config, move |_frame| {
            let _ = _tx.send(_frame);
        })
        .expect("Failed to initialize h264 decoder");
        println!("h264 decoder initialized successfully");
    }

    #[test]
    fn init_h265_decoder() {
        let (_tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Hevc);
        let _decoder = Decoder::new(config, move |_frame| {
            let _ = _tx.send(_frame);
        })
        .expect("Failed to initialize h265 decoder");
        println!("h265 decoder initialized successfully");
    }

    #[test]
    fn init_av1_decoder() {
        let (_tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Av1);
        let _decoder = Decoder::new(config, move |_frame| {
            let _ = _tx.send(_frame);
        })
        .expect("Failed to initialize av1 decoder");
        println!("av1 decoder initialized successfully");
    }

    #[test]
    fn init_vp8_decoder() {
        let (_tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Vp8);
        let _decoder = Decoder::new(config, move |_frame| {
            let _ = _tx.send(_frame);
        })
        .expect("Failed to initialize vp8 decoder");
        println!("vp8 decoder initialized successfully");
    }

    #[test]
    fn init_vp9_decoder() {
        let (_tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::Vp9);
        let _decoder = Decoder::new(config, move |_frame| {
            let _ = _tx.send(_frame);
        })
        .expect("Failed to initialize vp9 decoder");
        println!("vp9 decoder initialized successfully");
    }

    #[test]
    fn test_multiple_decoders() {
        let config = test_decoder_config(DecoderCodec::Hevc);
        let (_tx1, _rx1) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let _decoder1 = Decoder::new(config.clone(), move |_frame| {
            let _ = _tx1.send(_frame);
        })
        .expect("Failed to initialize first h265 decoder");

        let (_tx2, _rx2) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let _decoder2 = Decoder::new(config, move |_frame| {
            let _ = _tx2.send(_frame);
        })
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
        let decoder = Decoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
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

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y 平面と UV 平面のデータサイズを確認
        assert_eq!(frame.y_plane().len(), frame.y_stride() * frame.height());
        assert_eq!(
            frame.uv_plane().len(),
            frame.uv_stride() * frame.height().div_ceil(2)
        );

        // ストライドが幅以上であることを確認（GPU アラインメントのため）
        assert!(frame.y_stride() >= frame.width());
        assert!(frame.uv_stride() >= frame.width());

        // 黒画面なので、Y 成分は 16 付近、UV 成分は 128 付近の値になることを確認
        let y_data = frame.y_plane();
        let uv_data = frame.uv_plane();

        // Y 成分の平均値をチェック（完全な黒は 16）
        let y_avg = y_data.iter().map(|&x| x as u32).sum::<u32>() / y_data.len() as u32;
        assert!(
            (10..=30).contains(&y_avg),
            "Y average should be around 16 for black, got {}",
            y_avg
        );

        // UV 成分の平均値をチェック
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            (70..=140).contains(&uv_avg),
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded H.265 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);

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
        let decoder = Decoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
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

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y 平面と UV 平面のデータサイズを確認
        assert_eq!(frame.y_plane().len(), frame.y_stride() * frame.height());
        assert_eq!(
            frame.uv_plane().len(),
            frame.uv_stride() * frame.height().div_ceil(2)
        );

        // ストライドが幅以上であることを確認（GPU アラインメントのため）
        assert!(frame.y_stride() >= frame.width());
        assert!(frame.uv_stride() >= frame.width());

        // 黒画面なので、Y 成分は 16 付近、UV 成分は 128 付近の値になることを確認
        let y_data = frame.y_plane();
        let uv_data = frame.uv_plane();

        // Y 成分の平均値をチェック（完全な黒は 16）
        let y_avg = y_data.iter().map(|&x| x as u32).sum::<u32>() / y_data.len() as u32;
        assert!(
            (10..=30).contains(&y_avg),
            "Y average should be around 16 for black, got {}",
            y_avg
        );

        // UV 成分の平均値をチェック
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            (70..=140).contains(&uv_avg),
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded H.264 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);

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
        let decoder = Decoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
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

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y 平面と UV 平面のデータサイズを確認
        assert_eq!(frame.y_plane().len(), frame.y_stride() * frame.height());
        assert_eq!(
            frame.uv_plane().len(),
            frame.uv_stride() * frame.height().div_ceil(2)
        );

        // ストライドが幅以上であることを確認（GPU アラインメントのため）
        assert!(frame.y_stride() >= frame.width());
        assert!(frame.uv_stride() >= frame.width());

        // 黒画面なので、Y 成分は 16 付近、UV 成分は 128 付近の値になることを確認
        let y_data = frame.y_plane();
        let uv_data = frame.uv_plane();

        // Y 成分の平均値をチェック（完全な黒は 16）
        let y_avg = y_data.iter().map(|&x| x as u32).sum::<u32>() / y_data.len() as u32;
        assert!(
            (10..=30).contains(&y_avg),
            "Y average should be around 16 for black, got {}",
            y_avg
        );

        // UV 成分の平均値をチェック
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            (70..=140).contains(&uv_avg),
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded AV1 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);

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
        let decoder = Decoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
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

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y 平面と UV 平面のデータサイズを確認
        assert_eq!(frame.y_plane().len(), frame.y_stride() * frame.height());
        assert_eq!(
            frame.uv_plane().len(),
            frame.uv_stride() * frame.height().div_ceil(2)
        );

        // ストライドが幅以上であることを確認（GPU アラインメントのため）
        assert!(frame.y_stride() >= frame.width());
        assert!(frame.uv_stride() >= frame.width());

        // 黒画面なので、Y 成分は 16 付近、UV 成分は 128 付近の値になることを確認
        let y_data = frame.y_plane();
        let uv_data = frame.uv_plane();

        // Y 成分の平均値をチェック（完全な黒は 16）
        let y_avg = y_data.iter().map(|&x| x as u32).sum::<u32>() / y_data.len() as u32;
        assert!(
            (10..=30).contains(&y_avg),
            "Y average should be around 16 for black, got {}",
            y_avg
        );

        // UV 成分の平均値をチェック
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            (70..=140).contains(&uv_avg),
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded VP8 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);

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
        let decoder = Decoder::new(config, move |frame| {
            let _ = tx.send(frame);
        })
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

        assert_eq!(frame.width(), 640);
        assert_eq!(frame.height(), 480);

        // Y 平面と UV 平面のデータサイズを確認
        assert_eq!(frame.y_plane().len(), frame.y_stride() * frame.height());
        assert_eq!(
            frame.uv_plane().len(),
            frame.uv_stride() * frame.height().div_ceil(2)
        );

        // ストライドが幅以上であることを確認（GPU アラインメントのため）
        assert!(frame.y_stride() >= frame.width());
        assert!(frame.uv_stride() >= frame.width());

        // 黒画面なので、Y 成分は 16 付近、UV 成分は 128 付近の値になることを確認
        let y_data = frame.y_plane();
        let uv_data = frame.uv_plane();

        // Y 成分の平均値をチェック（完全な黒は 16）
        let y_avg = y_data.iter().map(|&x| x as u32).sum::<u32>() / y_data.len() as u32;
        assert!(
            (10..=30).contains(&y_avg),
            "Y average should be around 16 for black, got {}",
            y_avg
        );

        // UV 成分の平均値をチェック
        let uv_avg = uv_data.iter().map(|&x| x as u32).sum::<u32>() / uv_data.len() as u32;
        assert!(
            (70..=140).contains(&uv_avg),
            "UV average should be in reasonable range for the encoded frame, got {}",
            uv_avg
        );

        println!(
            "Successfully decoded VP9 black frame: {}x{} (stride: {})",
            frame.width(),
            frame.height(),
            frame.y_stride()
        );
        println!("Y average: {}, UV average: {}", y_avg, uv_avg);

        drop(decoder);
    }
}
