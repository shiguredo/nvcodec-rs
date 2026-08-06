use std::collections::VecDeque;
use std::ffi::c_void;
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

    /// 符号化解像度の最大幅 (cuvidReconfigureDecoder による動的解像度変更で使用)
    ///
    /// max_coded_height と両方指定した場合のみ reconfigure が有効になる。
    /// None の場合は従来どおりシーケンス変更ごとにデコーダーを破棄して再作成する
    pub max_coded_width: Option<u32>,

    /// 符号化解像度の最大高さ (cuvidReconfigureDecoder による動的解像度変更で使用)
    ///
    /// max_coded_width と両方指定した場合のみ reconfigure が有効になる。
    /// None の場合は従来どおりシーケンス変更ごとにデコーダーを破棄して再作成する
    pub max_coded_height: Option<u32>,
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
    max_coded_width: Option<u32>,
    max_coded_height: Option<u32>,
    // パーサ作成時に指定した ulMaxNumDecodeSurfaces
    // decoder の ulNumDecodeSurfaces を codec 別推奨値に引き上げる際、
    // この値を超えないように clamp する用途で保持する
    max_num_decode_surfaces: u32,
    // cuvidReconfigureDecoder の適用可否判定用のベースライン
    // 直近の create / reconfigure 時のコーデック情報を保存する
    reconfigure_baseline: ReconfigureBaseline,
    // cuvidCreateDecoder 呼び出し時に確定した出力ジオメトリを保存する
    //
    // 以降の cuvidReconfigureDecoder では ulTargetWidth / ulTargetHeight と
    // display_area を「作成時に確定した値」に固定して渡す必要がある
    // (縮小方向の解像度変更で ulTargetWidth / ulTargetHeight を新しい coded サイズに
    //  下げると、既に allocate 済みの出力サーフェスとの不整合により
    //  cuvidDecodePicture が CUDA_ERROR_INVALID_VALUE を返すため)
    create_geometry: DecoderCreateGeometry,
}

/// cuvidCreateDecoder 呼び出し時に確定した出力ジオメトリ
///
/// NVIDIA 公式サンプル (NvDecoder::ReconfigureDecoder) の挙動に合わせるため、
/// 以降の cuvidReconfigureDecoder では target / display_area をここに保存した
/// 「作成時の値」に固定して渡す
struct DecoderCreateGeometry {
    target_width: u32,
    target_height: u32,
    // display_area は CUVIDDECODECREATEINFO / CUVIDRECONFIGUREDECODERINFO とも
    // c_short (i16) で表現されるため i16 で保持する
    display_left: i16,
    display_top: i16,
    display_right: i16,
    display_bottom: i16,
}

/// cuvidReconfigureDecoder の適用可否判定用のベースライン
///
/// 直近の create / reconfigure 時のコーデック情報を保存する
struct ReconfigureBaseline {
    codec: sys::cudaVideoCodec,
    chroma_format: u32,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    progressive_sequence: u8,
}

impl ReconfigureBaseline {
    /// CUVIDEOFORMAT からベースラインを保存する
    fn from_format(format: &sys::CUVIDEOFORMAT) -> Self {
        Self {
            codec: format.codec,
            chroma_format: format.chroma_format,
            bit_depth_luma_minus8: format.bit_depth_luma_minus8,
            bit_depth_chroma_minus8: format.bit_depth_chroma_minus8,
            progressive_sequence: format.progressive_sequence,
        }
    }

    /// ベースラインからコーデック情報が変化したかを判定する
    ///
    /// cuvidReconfigureDecoder は same codec 限定のため、
    /// コーデック情報が変化した場合は reconfigure を使えない
    fn changed(&self, format: &sys::CUVIDEOFORMAT) -> bool {
        self.codec != format.codec
            || self.chroma_format != format.chroma_format
            || self.bit_depth_luma_minus8 != format.bit_depth_luma_minus8
            || self.bit_depth_chroma_minus8 != format.bit_depth_chroma_minus8
            || self.progressive_sequence != format.progressive_sequence
    }
}

unsafe impl Send for DecoderState {}

impl DecoderState {
    /// 指定されたコーデック設定でデコーダーインスタンスを生成する
    fn new(config: DecoderConfig) -> Result<Box<Self>, Error> {
        validate_max_coded_size(config.max_coded_width, config.max_coded_height)?;
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
                max_coded_width: config.max_coded_width,
                max_coded_height: config.max_coded_height,
                max_num_decode_surfaces: config.max_num_decode_surfaces,
                // 判定用ベースラインは初回のシーケンスコールバックで上書きされるため
                // ここでの初期値は意味を持たない
                reconfigure_baseline: ReconfigureBaseline::from_format(&std::mem::zeroed()),
                // 作成時ジオメトリも初回 cuvidCreateDecoder 呼び出しで上書きされるため
                // ここでの初期値は意味を持たない
                create_geometry: DecoderCreateGeometry {
                    target_width: 0,
                    target_height: 0,
                    display_left: 0,
                    display_top: 0,
                    display_right: 0,
                    display_bottom: 0,
                },
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

/// デコード結果を通知するためのハンドラー
///
/// デコード処理が完了するたびに [`DecodeHandler::on_decoded`] が呼ばれる。
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
pub struct Decoder<H: DecodeHandler> {
    job_tx: SyncSender<Job<H::UserData>>,
    worker: Option<JoinHandle<()>>,
}

impl<H: DecodeHandler> Decoder<H> {
    /// デコーダーを生成し、内部ワーカースレッドを起動する
    pub fn new(config: DecoderConfig, handler: H) -> Result<Self, Error> {
        let (job_tx, job_rx) = mpsc::sync_channel::<Job<H::UserData>>(4);

        let state = DecoderState::new(config)?;

        let worker = std::thread::Builder::new()
            .name("nvcodec-decoder".into())
            .spawn(move || {
                run_worker(state, handler, job_rx);
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
    /// デコードが完了すると、コンストラクタで渡したコールバックハンドラが呼び出される。
    pub fn decode(&self, data: &[u8], user_data: H::UserData) -> Result<(), Error> {
        self.job_tx
            .send(Job::Decode {
                data: data.to_vec(),
                user_data,
            })
            .map_err(|_| Error::new_custom("decode", "decoder worker thread has terminated"))
    }

    /// 送信済みの未完了フレームがすべて完了するまで待機する
    ///
    /// すべての pending フレームのコールバックハンドラが呼び出された後、このメソッドが戻る。
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

fn handle_video_sequence_inner(
    state: &mut DecoderState,
    format: &sys::CUVIDEOFORMAT,
) -> Result<i32, Error> {
    // display_area の検証は SDK 呼び出しより先に行う
    // 検証失敗時にデコーダーを破棄済みのままにしないため
    validate_display_area(format)?;

    // 最大解像度の超過を SDK 呼び出しより先に検証する
    // 初回コールバックか 2 回目以降かによらず、常に検証する
    if let (Some(max_width), Some(max_height)) = (state.max_coded_width, state.max_coded_height)
        && (format.coded_width > max_width || format.coded_height > max_height)
    {
        return Err(Error::new_custom(
            "handle_video_sequence",
            format!(
                "coded size ({}x{}) exceeds max_coded_width / max_coded_height ({}x{})",
                format.coded_width, format.coded_height, max_width, max_height
            ),
        ));
    }

    if state.decoder.is_null() {
        // 初回コールバックではデコーダーを新規作成する
        // 最大解像度が指定されている場合は ulMaxWidth / ulMaxHeight に設定して
        // 以降の cuvidReconfigureDecoder による in-place 再構成を可能にする
        create_decoder(state, format)?;
        save_reconfigure_baseline(state, format);
    } else if state.max_coded_width.is_none() || state.max_coded_height.is_none() {
        // 最大解像度が分からない場合は従来どおり破棄して再作成する
        // この経路では判定用ベースラインは使わないため保存値の更新は不要
        destroy_and_recreate_decoder(state, format)?;
    } else if state.reconfigure_baseline.changed(format) {
        // コーデック / クロマフォーマット / ビット深度 / progressive のいずれかが変化した場合
        // cuvidReconfigureDecoder は same codec 限定のため破棄して再作成する
        // 最大解像度は引き続き設定し、判定用ベースラインも更新して
        // 次回以降 reconfigure 経路に戻れるようにする
        destroy_and_recreate_decoder(state, format)?;
        save_reconfigure_baseline(state, format);
    } else {
        // それ以外は cuvidReconfigureDecoder で in-place に再構成する
        // パーサーと共有するコンテキストロックを使用する
        //
        // ulTargetWidth / ulTargetHeight と display_area は
        // 作成時に確定した値 (state.create_geometry) をそのまま渡す
        // (新しい coded サイズをここに渡すと、既に作成時サイズで allocate された
        //  出力サーフェスとの不整合により cuvidDecodePicture が縮小時に
        //  CUDA_ERROR_INVALID_VALUE を返す。NVIDIA 公式サンプル
        //  NvDecoder::ReconfigureDecoder も同様に作成時サイズを維持している)
        state.lib.with_context(state.ctx, || {
            let mut reconfigure_info: sys::CUVIDRECONFIGUREDECODERINFO =
                unsafe { std::mem::zeroed() };
            reconfigure_info.ulWidth = format.coded_width;
            reconfigure_info.ulHeight = format.coded_height;
            reconfigure_info.ulTargetWidth = state.create_geometry.target_width;
            reconfigure_info.ulTargetHeight = state.create_geometry.target_height;
            reconfigure_info.display_area.left = state.create_geometry.display_left;
            reconfigure_info.display_area.top = state.create_geometry.display_top;
            reconfigure_info.display_area.right = state.create_geometry.display_right;
            reconfigure_info.display_area.bottom = state.create_geometry.display_bottom;
            // ulNumDecodeSurfaces は create 時と同じ codec 別推奨値を渡す
            // (parser 報告の min では DPB 不足で cuvidDecodePicture が失敗するため)
            reconfigure_info.ulNumDecodeSurfaces =
                effective_num_decode_surfaces(format, state.max_num_decode_surfaces);
            state
                .lib
                .cuvid_reconfigure_decoder(state.decoder, &mut reconfigure_info)
        })?;
    }

    update_decoder_dimensions(state, format);

    // シーケンスコールバックの戻り値は decoder の ulNumDecodeSurfaces と
    // 同じ値でなければならない (parser がこの値で curr_pic_idx を割り当てるため)
    Ok(effective_num_decode_surfaces(format, state.max_num_decode_surfaces) as i32)
}

/// デコーダーを新規作成する
///
/// ulMaxWidth / ulMaxHeight には max_coded_width / max_coded_height が
/// 指定されている場合はその値を、None の場合は現在の coded_width / coded_height を設定する
fn create_decoder(state: &mut DecoderState, format: &sys::CUVIDEOFORMAT) -> Result<(), Error> {
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
    // ulNumDecodeSurfaces は codec 別の推奨値を採用する
    // (parser 報告の min_num_decode_surfaces では HEVC/VP9/AV1 で DPB 不足になり
    //  cuvidReconfigureDecoder 後の cuvidDecodePicture が失敗するため)
    // なお reconfigure で ulNumDecodeSurfaces を後から増やすことはできないため、
    // 最初の create で十分な値を確保しておく必要がある
    create_info.ulNumDecodeSurfaces =
        effective_num_decode_surfaces(format, state.max_num_decode_surfaces) as u64;
    create_info.ulWidth = format.coded_width as u64;
    create_info.ulHeight = format.coded_height as u64;
    create_info.ulMaxWidth = state.max_coded_width.unwrap_or(format.coded_width) as u64;
    create_info.ulMaxHeight = state.max_coded_height.unwrap_or(format.coded_height) as u64;
    create_info.ulTargetWidth = format.coded_width as u64;
    create_info.ulTargetHeight = format.coded_height as u64;

    // display_area は以降の cuvidReconfigureDecoder でも同じ値を再度渡す必要があるため
    // ここで明示的に設定する (i32 → i16 のキャストは display_area の検証で
    // 負値 / 逆転を弾いており、実用上の解像度は i16 の上限を超えないため安全)
    create_info.display_area.left = format.display_area.left as i16;
    create_info.display_area.top = format.display_area.top as i16;
    create_info.display_area.right = format.display_area.right as i16;
    create_info.display_area.bottom = format.display_area.bottom as i16;

    // パーサーと共有するコンテキストロックを使用
    create_info.vidLock = state.ctx_lock;

    state.lib.with_context(state.ctx, || {
        state
            .lib
            .cuvid_create_decoder(&mut state.decoder, &mut create_info)
    })?;

    // 作成時ジオメトリを保存する
    // 以降の cuvidReconfigureDecoder では target / display_area をこの値に固定して渡す
    state.create_geometry = DecoderCreateGeometry {
        target_width: format.coded_width,
        target_height: format.coded_height,
        display_left: create_info.display_area.left,
        display_top: create_info.display_area.top,
        display_right: create_info.display_area.right,
        display_bottom: create_info.display_area.bottom,
    };

    Ok(())
}

/// 既存デコーダーを破棄してから再作成する
fn destroy_and_recreate_decoder(
    state: &mut DecoderState,
    format: &sys::CUVIDEOFORMAT,
) -> Result<(), Error> {
    state
        .lib
        .with_context(state.ctx, || state.lib.cuvid_destroy_decoder(state.decoder))?;
    state.decoder = ptr::null_mut();
    create_decoder(state, format)
}

/// コーデック別の推奨デコードサーフェス数を返す
///
/// NVIDIA 公式サンプル NvDecoder::GetNumDecodeSurfaces に準拠する。
/// 参照フレーム数の多い HEVC / VP9 / AV1 では parser 報告値の
/// `min_num_decode_surfaces` (通常 8-9) では DPB が不足して
/// 縮小方向の reconfigure 直後の cuvidDecodePicture が
/// CUDA_ERROR_INVALID_VALUE を返すため、コーデック仕様の最大参照フレーム数に
/// 余裕を加えた値を使う。
///
/// 実際に create / reconfigure に渡す値は
/// `max(min_num_decode_surfaces, get_codec_num_decode_surfaces(...))`。
fn get_codec_num_decode_surfaces(codec: sys::cudaVideoCodec) -> u32 {
    // NVIDIA サンプル (NvDecoder.cpp) の GetNumDecodeSurfaces と同じ値を採用する
    // AV1 はサンプルでは default (8) だが、仕様上 8 参照 + 現在フレーム = 9 必要なため
    // 余裕を持たせて VP9 相当の 12 を採用する
    match codec {
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_VP9 => 12,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC => 20,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_H264 => 20,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_AV1 => 12,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_VP8 => 8,
        c if c == sys::cudaVideoCodec_enum_cudaVideoCodec_JPEG => 1,
        // それ以外は NVIDIA サンプルの default 値
        _ => 8,
    }
}

/// format と codec からデコードサーフェス数を決定する
///
/// parser 報告の最小値と codec 別推奨値の大きい方を採用し、
/// パーサ作成時に指定した `ulMaxNumDecodeSurfaces` を上限として clamp する
/// (シーケンスコールバックの戻り値がパーサ上限を超えると parser が
///  想定外の curr_pic_idx を生成する可能性があるため)
fn effective_num_decode_surfaces(format: &sys::CUVIDEOFORMAT, max_allowed: u32) -> u32 {
    let min = format.min_num_decode_surfaces as u32;
    let codec_recommended = get_codec_num_decode_surfaces(format.codec);
    min.max(codec_recommended).min(max_allowed.max(min))
}

/// max_coded_width / max_coded_height を検証する
///
/// 両方 Some か両方 None のどちらかでなければならない
/// (片方だけ Some は reconfigure が無効になるだけでなく、
///  create 時に ulMaxWidth < ulWidth の矛盾した値が SDK に渡りうるため)
fn validate_max_coded_size(
    max_coded_width: Option<u32>,
    max_coded_height: Option<u32>,
) -> Result<(), Error> {
    if max_coded_width.is_some() != max_coded_height.is_some() {
        return Err(Error::new_custom(
            "Decoder::new",
            "max_coded_width and max_coded_height must be both Some or both None",
        ));
    }
    Ok(())
}

/// display_area を検証する
///
/// display_area は signed 整数のため、壊れたストリームで負値になる可能性がある
/// state.width / state.height の計算に使う right - left などが破綻しないことを保証する
fn validate_display_area(format: &sys::CUVIDEOFORMAT) -> Result<(), Error> {
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
    Ok(())
}

/// reconfigure 適用可否判定用のベースラインを保存する
fn save_reconfigure_baseline(state: &mut DecoderState, format: &sys::CUVIDEOFORMAT) {
    state.reconfigure_baseline = ReconfigureBaseline::from_format(format);
}

/// デコード結果の寸法を更新する
///
/// display_area は handle_video_sequence_inner の先頭で検証済みのため安全に計算できる
fn update_decoder_dimensions(state: &mut DecoderState, format: &sys::CUVIDEOFORMAT) {
    state.width = (format.display_area.right - format.display_area.left) as u32;
    state.height = (format.display_area.bottom - format.display_area.top) as u32;
    state.surface_width = format.coded_width;
    state.surface_height = format.coded_height;
}

unsafe extern "C" fn handle_video_sequence(
    user_data: *mut c_void,
    format: *mut sys::CUVIDEOFORMAT,
) -> i32 {
    if user_data.is_null() || format.is_null() {
        return 0;
    }
    let state = unsafe { &mut *(user_data as *mut DecoderState) };
    match handle_video_sequence_inner(state, unsafe { &*format }) {
        Ok(val) => val,
        Err(e) => {
            let _ = state.frame_tx.send(Err(e));
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
    match handle_picture_decode_inner(state, unsafe { &*pic_params }) {
        Ok(()) => 1,
        Err(e) => {
            let _ = state.frame_tx.send(Err(e));
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
    let state = unsafe { &*(user_data as *const DecoderState) };
    match handle_picture_display_inner(state, unsafe { &*disp_info }) {
        Ok(()) => 1,
        Err(e) => {
            let _ = state.frame_tx.send(Err(e));
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

fn run_worker<H>(mut state: Box<DecoderState>, mut handler: H, job_rx: Receiver<Job<H::UserData>>)
where
    H: DecodeHandler,
{
    let mut pending_user_data: VecDeque<H::UserData> = VecDeque::new();

    loop {
        match job_rx.recv() {
            Ok(Job::Decode { data, user_data }) => {
                if let Err(e) = state.decode(&data) {
                    handler.on_decoded(Err(e.into()));
                    continue;
                }

                pending_user_data.push_back(user_data);
                drain_frames(&mut state, &mut handler, &mut pending_user_data);
            }
            Ok(Job::Flush { done }) => {
                let _ = state.send_eos();

                drain_frames(&mut state, &mut handler, &mut pending_user_data);

                let _ = done.send(());
            }
            Ok(Job::Terminate) | Err(_) => {
                // 残っている非同期処理を完了させる
                let _ = state.send_eos();

                drain_frames(&mut state, &mut handler, &mut pending_user_data);

                // state の Drop がここで走り、CUDA リソースが解放される
                return;
            }
        }
    }
}

fn drain_frames<H>(
    state: &mut DecoderState,
    handler: &mut H,
    pending_user_data: &mut VecDeque<H::UserData>,
) where
    H: DecodeHandler,
{
    loop {
        match state.next_frame() {
            Ok(None) => {
                // 結果が存在しなくなったなら終了
                break;
            }
            Ok(Some(raw)) => {
                if let Some(user_data) = pending_user_data.pop_front() {
                    handler.on_decoded(Ok(DecodedFrame {
                        width: raw.width,
                        height: raw.height,
                        pitch: raw.pitch,
                        data: raw.data,
                        user_data,
                    }));
                } else {
                    // デコード結果が存在するのに対応するユーザーデータが存在しない
                    // これは通常あり得ないはずだけど、エラーを取りこぼさない為に
                    // エラーのコールバックハンドラを呼ぶ
                    handler.on_decoded(Err(
                        Error::new_custom("drain_frames", "missing user data").into()
                    ));
                    break;
                }
            }
            // エラーが起きたら全てのユーザーデータを削除して
            // コールバックハンドラを呼ぶ
            Err(e) => {
                pending_user_data.clear();
                handler.on_decoded(Err(e.into()));
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
            max_coded_width: None,
            max_coded_height: None,
        }
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
        use std::mem::ManuallyDrop;

        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::H264);

        let mut decoder = ManuallyDrop::new(
            Decoder::new(
                config,
                FnDecodeHandler::new(move |frame| {
                    let _ = tx.send(frame);
                }),
            )
            .unwrap(),
        );

        unsafe { ManuallyDrop::drop(&mut decoder) };

        let result = decoder.decode(&[], ());
        assert_eq!(
            result.unwrap_err().to_string(),
            "decode() failed: decoder worker thread has terminated"
        );

        unsafe {
            ManuallyDrop::drop(&mut decoder);
        }
    }

    #[test]
    fn test_flush_after_decoder_worker_terminated() {
        use std::mem::ManuallyDrop;

        let (tx, _rx) = mpsc::sync_channel::<Result<DecodedFrame<()>, Error>>(4);
        let config = test_decoder_config(DecoderCodec::H264);

        let mut decoder = ManuallyDrop::new(
            Decoder::new(
                config,
                FnDecodeHandler::new(move |frame| {
                    let _ = tx.send(frame);
                }),
            )
            .unwrap(),
        );

        unsafe { ManuallyDrop::drop(&mut decoder) };

        let result = decoder.flush();
        assert_eq!(
            result.unwrap_err().to_string(),
            "flush() failed: send failed"
        );

        unsafe {
            ManuallyDrop::drop(&mut decoder);
        }
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

    /// テスト用の CUVIDEOFORMAT を生成する
    fn test_video_format(coded_width: u32, coded_height: u32) -> sys::CUVIDEOFORMAT {
        let mut format: sys::CUVIDEOFORMAT = unsafe { std::mem::zeroed() };
        format.codec = sys::cudaVideoCodec_enum_cudaVideoCodec_H264;
        format.coded_width = coded_width;
        format.coded_height = coded_height;
        format.display_area.left = 0;
        format.display_area.top = 0;
        format.display_area.right = coded_width as i32;
        format.display_area.bottom = coded_height as i32;
        format.chroma_format = sys::cudaVideoChromaFormat_enum_cudaVideoChromaFormat_420;
        format.min_num_decode_surfaces = 2;
        format.progressive_sequence = 1;
        format
    }

    #[test]
    fn test_validate_max_coded_size_both_some() {
        // 両方 Some は検証を通過する
        validate_max_coded_size(Some(320), Some(240)).expect("both Some should pass");
    }

    #[test]
    fn test_validate_max_coded_size_both_none() {
        // 両方 None は検証を通過する
        validate_max_coded_size(None, None).expect("both None should pass");
    }

    #[test]
    fn test_validate_max_coded_size_width_only() {
        // 幅だけの指定は検証に失敗する
        let error = validate_max_coded_size(Some(320), None).expect_err("width only should fail");
        assert!(error.to_string().contains("both Some or both None"));
    }

    #[test]
    fn test_validate_max_coded_size_height_only() {
        // 高さだけの指定は検証に失敗する
        let error = validate_max_coded_size(None, Some(240)).expect_err("height only should fail");
        assert!(error.to_string().contains("both Some or both None"));
    }

    #[test]
    fn test_validate_display_area_valid() {
        // 有効な display_area は検証を通過する
        let format = test_video_format(320, 240);
        validate_display_area(&format).expect("valid display_area should pass");
    }

    #[test]
    fn test_validate_display_area_negative_left() {
        // 負の left は壊れたストリームを示すため検証に失敗する
        let mut format = test_video_format(320, 240);
        format.display_area.left = -1;
        assert!(validate_display_area(&format).is_err());
    }

    #[test]
    fn test_validate_display_area_negative_top() {
        // 負の top は壊れたストリームを示すため検証に失敗する
        let mut format = test_video_format(320, 240);
        format.display_area.top = -1;
        assert!(validate_display_area(&format).is_err());
    }

    #[test]
    fn test_validate_display_area_reversed() {
        // right <= left は壊れたストリームを示すため検証に失敗する
        let mut format = test_video_format(320, 240);
        format.display_area.right = 0;
        format.display_area.left = 100;
        assert!(validate_display_area(&format).is_err());
    }

    #[test]
    fn test_validate_display_area_bottom_reversed() {
        // bottom <= top は壊れたストリームを示すため検証に失敗する
        let mut format = test_video_format(320, 240);
        format.display_area.bottom = 100;
        format.display_area.top = 200;
        assert!(validate_display_area(&format).is_err());
    }

    #[test]
    fn test_validate_display_area_exceeds_coded_size() {
        // display_area が coded size を超えると検証に失敗する
        let mut format = test_video_format(320, 240);
        format.display_area.right = 321;
        assert!(validate_display_area(&format).is_err());
    }

    #[test]
    fn test_validate_display_area_exceeds_coded_size_bottom() {
        // display_area が coded size を超えると検証に失敗する
        let mut format = test_video_format(320, 240);
        format.display_area.bottom = 241;
        assert!(validate_display_area(&format).is_err());
    }

    #[test]
    fn test_reconfigure_baseline_no_change() {
        // コーデック情報が同じ場合は再作成不要と判定する
        let format = test_video_format(320, 240);
        let baseline = ReconfigureBaseline::from_format(&format);
        assert!(!baseline.changed(&format));
    }

    #[test]
    fn test_reconfigure_baseline_resolution_change_only() {
        // 解像度のみの変化では再作成不要と判定する (reconfigure 対象)
        let format = test_video_format(320, 240);
        let baseline = ReconfigureBaseline::from_format(&format);
        let mut smaller = format;
        smaller.coded_width = 160;
        smaller.coded_height = 120;
        assert!(!baseline.changed(&smaller));
    }

    #[test]
    fn test_reconfigure_baseline_codec_changed() {
        // コーデックの変化は reconfigure 不可のため再作成必要と判定する
        let mut format = test_video_format(320, 240);
        let baseline = ReconfigureBaseline::from_format(&format);
        format.codec = sys::cudaVideoCodec_enum_cudaVideoCodec_HEVC;
        assert!(baseline.changed(&format));
    }

    #[test]
    fn test_reconfigure_baseline_chroma_format_changed() {
        // クロマフォーマットの変化は reconfigure 不可のため再作成必要と判定する
        let mut format = test_video_format(320, 240);
        let baseline = ReconfigureBaseline::from_format(&format);
        format.chroma_format = sys::cudaVideoChromaFormat_enum_cudaVideoChromaFormat_420 + 1;
        assert!(baseline.changed(&format));
    }

    #[test]
    fn test_reconfigure_baseline_bit_depth_luma_changed() {
        // ルマのビット深度の変化は reconfigure 不可のため再作成必要と判定する
        let mut format = test_video_format(320, 240);
        let baseline = ReconfigureBaseline::from_format(&format);
        format.bit_depth_luma_minus8 = 2;
        assert!(baseline.changed(&format));
    }

    #[test]
    fn test_reconfigure_baseline_bit_depth_chroma_changed() {
        // クロマのビット深度の変化は reconfigure 不可のため再作成必要と判定する
        let mut format = test_video_format(320, 240);
        let baseline = ReconfigureBaseline::from_format(&format);
        format.bit_depth_chroma_minus8 = 2;
        assert!(baseline.changed(&format));
    }

    #[test]
    fn test_reconfigure_baseline_progressive_changed() {
        // progressive / インターレースの変化は reconfigure 不可のため再作成必要と判定する
        let mut format = test_video_format(320, 240);
        let baseline = ReconfigureBaseline::from_format(&format);
        format.progressive_sequence = 0;
        assert!(baseline.changed(&format));
    }

    /// Annex-B 形式のビットストリームをアクセスユニット (フレーム) 単位に分割する
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
            let size = u32::from_le_bytes(data[offset..offset + 4].try_into().expect("infallible"))
                as usize;
            offset += 12;
            frames.push(&data[offset..offset + size]);
            offset += size;
        }
        frames
    }

    /// テストデータを 1 フレームずつデコードしてフレームとエラーを収集する
    fn decode_resolution_change_data(
        codec: DecoderCodec,
        frames: &[&[u8]],
        max_coded_width: Option<u32>,
        max_coded_height: Option<u32>,
    ) -> (Vec<DecodedFrame<()>>, Vec<Error>) {
        let config = DecoderConfig {
            codec,
            device_id: 0,
            max_num_decode_surfaces: 20,
            max_display_delay: 0,
            surface_format: SurfaceFormat::Nv12,
            max_coded_width,
            max_coded_height,
        };
        let (tx, rx) = mpsc::channel();
        let decoder = Decoder::new(
            config,
            FnDecodeHandler::new(move |frame| {
                let _ = tx.send(frame);
            }),
        )
        .expect("Failed to create decoder");

        for frame in frames {
            // シーケンスコールバックのエラーはチャネル経由で通知されるため
            // decode の戻り値は確認しない
            let _ = decoder.decode(frame, ());
        }
        let _ = decoder.flush();

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
        (decoded_frames, errors)
    }

    /// 解像度変化ストリームのデコード結果を検証する
    ///
    /// 全 45 フレームが 320x240 x30 と 256x160 x15 でデコードされることを確認する
    fn assert_resolution_change_frames(
        codec: DecoderCodec,
        data: &'static [u8],
        frames: &[&[u8]],
        max_coded_width: Option<u32>,
        max_coded_height: Option<u32>,
    ) {
        let (decoded_frames, errors) =
            decode_resolution_change_data(codec, frames, max_coded_width, max_coded_height);

        // エラーが 1 件も通知されないことを確認する
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");

        // 全フレームがデコードされることを確認する
        // max_coded_* を指定した場合は cuvidReconfigureDecoder による in-place 再構成で
        // フレームロスが発生しない。指定しない場合は destroy+create のため
        // 再作成時に in-flight フレームが失われる (フレーム数が減る)
        assert_eq!(
            decoded_frames.len(),
            frames.len(),
            "decoded frame count should match input frame count (codec: {codec:?}, data: {data:?})"
        );

        // 各フレームのサイズを検証する
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
        assert_eq!(frames.len(), 45, "h264 frame count");

        // 先頭フレームには SPS (NAL type 7) が含まれる
        // 先頭フレームからパラメータセットが欠落するとデコードできないため
        // (nal_ref_idc の値はエンコーダーによって異なるため NAL type ビットのみで判定する)
        assert!(
            frames[0]
                .windows(4)
                .any(|w| w[..3] == [0, 0, 1] && (w[3] & 0x1f) == 7),
            "first frame should contain SPS"
        );
    }

    #[test]
    fn test_split_annexb_frames_h265() {
        // H.265 テストデータは 45 アクセスユニットに分割される
        let data = include_bytes!("../testdata/resolution-change/h265.h265");
        let frames = split_annexb_frames(data, |nal| nal >> 1 <= 31);
        assert_eq!(frames.len(), 45, "h265 frame count");

        // 先頭フレームには VPS (NAL type 32) が含まれる
        // 先頭フレームからパラメータセットが欠落するとデコードできないため
        assert!(
            frames[0].windows(4).any(|w| w == [0, 0, 1, 0x40]),
            "first frame should contain VPS"
        );
    }

    #[test]
    fn test_split_ivf_frames() {
        // IVF テストデータは 45 フレームに分割される
        let vp8_data = include_bytes!("../testdata/resolution-change/vp8.ivf");
        assert_eq!(split_ivf_frames(vp8_data).len(), 45, "vp8 frame count");
        let vp9_data = include_bytes!("../testdata/resolution-change/vp9.ivf");
        assert_eq!(split_ivf_frames(vp9_data).len(), 45, "vp9 frame count");
        let av1_data = include_bytes!("../testdata/resolution-change/av1.ivf");
        assert_eq!(split_ivf_frames(av1_data).len(), 45, "av1 frame count");
    }

    #[test]
    fn test_decode_h264_resolution_change_with_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定して H.264 の解像度変化ストリームをデコードする
        // 320x240 → 256x160 → 320x240 の変化を cuvidReconfigureDecoder で処理する
        let data = include_bytes!("../testdata/resolution-change/h264.h264");
        let frames = split_annexb_frames(data, |nal| (nal & 0x1f) == 1 || (nal & 0x1f) == 5);
        assert_eq!(frames.len(), 45, "h264");
        assert_resolution_change_frames(DecoderCodec::H264, data, &frames, Some(320), Some(240));
    }

    #[test]
    fn test_decode_h265_resolution_change_with_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定して H.265 の解像度変化ストリームをデコードする
        // 320x240 → 256x160 → 320x240 の変化を cuvidReconfigureDecoder で処理する
        let data = include_bytes!("../testdata/resolution-change/h265.h265");
        let frames = split_annexb_frames(data, |nal| nal >> 1 <= 31);
        assert_eq!(frames.len(), 45, "h265");
        assert_resolution_change_frames(DecoderCodec::Hevc, data, &frames, Some(320), Some(240));
    }

    #[test]
    fn test_decode_vp8_resolution_change_with_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定して VP8 の解像度変化ストリームをデコードする
        // 320x240 → 256x160 → 320x240 の変化を cuvidReconfigureDecoder で処理する
        let data = include_bytes!("../testdata/resolution-change/vp8.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "vp8");
        assert_resolution_change_frames(DecoderCodec::Vp8, data, &frames, Some(320), Some(240));
    }

    #[test]
    fn test_decode_vp9_resolution_change_with_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定して VP9 の解像度変化ストリームをデコードする
        // 320x240 → 256x160 → 320x240 の変化を cuvidReconfigureDecoder で処理する
        let data = include_bytes!("../testdata/resolution-change/vp9.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "vp9");
        assert_resolution_change_frames(DecoderCodec::Vp9, data, &frames, Some(320), Some(240));
    }

    #[test]
    fn test_decode_av1_resolution_change_with_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定して AV1 の解像度変化ストリームをデコードする
        // 320x240 → 256x160 → 320x240 の変化を cuvidReconfigureDecoder で処理する
        let data = include_bytes!("../testdata/resolution-change/av1.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "av1");
        assert_resolution_change_frames(DecoderCodec::Av1, data, &frames, Some(320), Some(240));
    }

    #[test]
    fn test_decode_h264_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合は従来どおり destroy+create で
        // 解像度変化に対応する
        let data = include_bytes!("../testdata/resolution-change/h264.h264");
        let frames = split_annexb_frames(data, |nal| (nal & 0x1f) == 1 || (nal & 0x1f) == 5);
        assert_eq!(frames.len(), 45, "h264");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::H264, &frames);
    }

    #[test]
    fn test_decode_h265_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合は従来どおり destroy+create で
        // 解像度変化に対応する
        let data = include_bytes!("../testdata/resolution-change/h265.h265");
        let frames = split_annexb_frames(data, |nal| nal >> 1 <= 31);
        assert_eq!(frames.len(), 45, "h265");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Hevc, &frames);
    }

    #[test]
    fn test_decode_vp8_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合は従来どおり destroy+create で
        // 解像度変化に対応する
        let data = include_bytes!("../testdata/resolution-change/vp8.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "vp8");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Vp8, &frames);
    }

    #[test]
    fn test_decode_vp9_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合は従来どおり destroy+create で
        // 解像度変化に対応する
        let data = include_bytes!("../testdata/resolution-change/vp9.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "vp9");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Vp9, &frames);
    }

    #[test]
    fn test_decode_av1_resolution_change_without_max_coded_width_height() {
        // max_coded_width / max_coded_height を指定しない場合は従来どおり destroy+create で
        // 解像度変化に対応する
        let data = include_bytes!("../testdata/resolution-change/av1.ivf");
        let frames = split_ivf_frames(data);
        assert_eq!(frames.len(), 45, "av1");
        assert_resolution_change_frames_destroy_and_recreate(DecoderCodec::Av1, &frames);
    }

    /// destroy+create 経路で 45 フレーム全てがデコードされることを確認する
    ///
    /// display_delay=0 のためシーケンス変更時に in-flight フレームが存在せず、
    /// フレームロスは発生しないことを期待する
    fn assert_resolution_change_frames_destroy_and_recreate(codec: DecoderCodec, frames: &[&[u8]]) {
        let (decoded_frames, errors) = decode_resolution_change_data(codec, frames, None, None);

        // エラーが 1 件も通知されないことを確認する
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");

        // destroy+create でも全フレームがデコードされる
        assert_eq!(
            decoded_frames.len(),
            frames.len(),
            "all frames should be decoded (codec: {codec:?}): {}",
            decoded_frames.len()
        );

        // 各フレームのサイズを検証する
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
    fn test_decode_resolution_change_exceeds_max_coded_width_height() {
        // max_coded_width / max_coded_height (160x120) を超える 320x240 のストリームをデコードすると
        // 初回のシーケンスコールバックでエラーが通知される
        let data = include_bytes!("../testdata/resolution-change/h264.h264");
        let frames = split_annexb_frames(data, |nal| (nal & 0x1f) == 1 || (nal & 0x1f) == 5);
        let (decoded_frames, errors) =
            decode_resolution_change_data(DecoderCodec::H264, &frames, Some(160), Some(120));

        // フレームは 1 件もデコードされない
        assert!(decoded_frames.is_empty(), "no frames should be decoded");

        // max 超過エラーが通知される
        assert!(
            errors.iter().any(|e| e
                .to_string()
                .contains("exceeds max_coded_width / max_coded_height")),
            "max_coded_width / max_coded_height error should be reported: {errors:?}"
        );
    }
}
