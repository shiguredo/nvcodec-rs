//! [NVCODEC] エンコーダーとデコーダー
//!
//! [NVCODEC]: https://developer.nvidia.com/video-codec-sdk
#![warn(missing_docs)]

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::path::Path;
use std::sync::{Arc, LazyLock};

mod codec_info;
mod decode;
mod dl;
mod encode;
mod error;
mod sys;

pub use codec_info::*;
pub use decode::{DecodedFrame, Decoder, DecoderCaps, DecoderCodec, DecoderConfig, SurfaceFormat};
pub use encode::{
    Av1EncoderConfig, Av1Profile, BufferFormat, CodecConfig, EncodeOptions, EncodedFrame, Encoder,
    EncoderCaps, EncoderCodec, EncoderConfig, H264EncoderConfig, H264Profile, HevcEncoderConfig,
    HevcProfile, PictureType, Preset, RateControlMode, ReconfigureParams, TuningInfo,
};
pub use error::Error;

/// ビルド時に参照したバージョン
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// CUDA ライブラリのラッパー構造体
#[derive(Debug, Clone)]
struct CudaLibrary {
    cuda_lib: Arc<dl::DynLib>,
    nvcuvid_lib: Arc<dl::DynLib>,
    nvenc_lib: Arc<dl::DynLib>,
}

impl CudaLibrary {
    /// CUDA ライブラリをロードし、必要な関数が利用可能かチェックする
    fn load() -> Result<Self, Error> {
        type Libraries = (Arc<dl::DynLib>, Arc<dl::DynLib>, Arc<dl::DynLib>);
        static LIBS: LazyLock<Result<Libraries, Error>> = LazyLock::new(|| {
            // CUDA ドライバーライブラリをロード
            let cuda_lib = dl::DynLib::open(Path::new("libcuda.so.1"))
                .map(Arc::new)
                .map_err(|_| {
                    Error::new_custom(
                        "CudaLibrary::load",
                        "failed to load CUDA library (libcuda.so.1 not found)",
                    )
                })?;

            // cuInit を呼び出して CUDA ドライバーを初期化
            let cu_init: unsafe extern "C" fn(u32) -> u32 = unsafe {
                cuda_lib
                    .get(b"cuInit")
                    .map_err(|_| Error::new_custom("CudaLibrary::load", "cuInit not found"))?
            };
            let flags = 0;
            let status = unsafe { cu_init(flags) };
            Error::check_cuda(status, "cuInit")?;

            // NVCUVID ライブラリをロード（デコード用）
            let nvcuvid_lib = dl::DynLib::open(Path::new("libnvcuvid.so.1"))
                .map(Arc::new)
                .map_err(|_| {
                    Error::new_custom(
                        "CudaLibrary::load",
                        "failed to load NVCUVID library (libnvcuvid.so.1 not found)",
                    )
                })?;

            // NVENC ライブラリをロード（エンコード用）
            let nvenc_lib = dl::DynLib::open(Path::new("libnvidia-encode.so.1"))
                .map(Arc::new)
                .map_err(|_| {
                    Error::new_custom(
                        "CudaLibrary::load",
                        "failed to load NVENC library (libnvidia-encode.so.1 not found)",
                    )
                })?;

            // 必要な関数が存在するか確認
            unsafe {
                // エラー関連
                let _: unsafe extern "C" fn(u32, *mut *const u8) -> u32 =
                    cuda_lib.get(b"cuGetErrorName").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuGetErrorName not found")
                    })?;

                let _: unsafe extern "C" fn(u32, *mut *const u8) -> u32 =
                    cuda_lib.get(b"cuGetErrorString").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuGetErrorString not found")
                    })?;

                // コンテキスト管理関連
                let _: unsafe extern "C" fn(*mut sys::CUcontext, u32, i32) -> u32 =
                    cuda_lib.get(b"cuCtxCreate_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuCtxCreate_v2 not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUcontext) -> u32 =
                    cuda_lib.get(b"cuCtxDestroy_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuCtxDestroy_v2 not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUcontext) -> u32 =
                    cuda_lib.get(b"cuCtxPushCurrent_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuCtxPushCurrent_v2 not found")
                    })?;

                let _: unsafe extern "C" fn(*mut sys::CUcontext) -> u32 =
                    cuda_lib.get(b"cuCtxPopCurrent_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuCtxPopCurrent_v2 not found")
                    })?;

                let _: unsafe extern "C" fn() -> u32 =
                    cuda_lib.get(b"cuCtxSynchronize").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuCtxSynchronize not found")
                    })?;

                // メモリ管理関連
                let _: unsafe extern "C" fn(*mut sys::CUdeviceptr, usize) -> u32 =
                    cuda_lib.get(b"cuMemAlloc_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuMemAlloc_v2 not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUdeviceptr) -> u32 =
                    cuda_lib.get(b"cuMemFree_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuMemFree_v2 not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUdeviceptr, *const c_void, usize) -> u32 =
                    cuda_lib.get(b"cuMemcpyHtoD_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuMemcpyHtoD_v2 not found")
                    })?;

                let _: unsafe extern "C" fn(*mut c_void, sys::CUdeviceptr, usize) -> u32 =
                    cuda_lib.get(b"cuMemcpyDtoH_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuMemcpyDtoH_v2 not found")
                    })?;

                // デバイス列挙関連
                let _: unsafe extern "C" fn(*mut c_int) -> u32 =
                    cuda_lib.get(b"cuDeviceGetCount").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuDeviceGetCount not found")
                    })?;

                let _: unsafe extern "C" fn(*mut c_int, c_int) -> u32 = cuda_lib
                    .get(b"cuDeviceGet")
                    .map_err(|_| Error::new_custom("CudaLibrary::load", "cuDeviceGet not found"))?;

                let _: unsafe extern "C" fn(*mut c_char, c_int, c_int) -> u32 =
                    cuda_lib.get(b"cuDeviceGetName").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuDeviceGetName not found")
                    })?;

                // 2D メモリコピー関連
                let _: unsafe extern "C" fn(
                    *mut sys::CUdeviceptr,
                    *mut usize,
                    usize,
                    usize,
                    c_uint,
                ) -> u32 = cuda_lib.get(b"cuMemAllocPitch_v2").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuMemAllocPitch_v2 not found")
                })?;

                let _: unsafe extern "C" fn(*const sys::CUDA_MEMCPY2D) -> u32 =
                    cuda_lib.get(b"cuMemcpy2D_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuMemcpy2D_v2 not found")
                    })?;

                // ストリーム管理関連
                let _: unsafe extern "C" fn(*mut sys::CUstream, c_uint) -> u32 =
                    cuda_lib.get(b"cuStreamCreate").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuStreamCreate not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUstream) -> u32 =
                    cuda_lib.get(b"cuStreamSynchronize").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuStreamSynchronize not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUstream) -> u32 =
                    cuda_lib.get(b"cuStreamDestroy_v2").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuStreamDestroy_v2 not found")
                    })?;

                // デコーダケーパビリティクエリ関連
                let _: unsafe extern "C" fn(*mut sys::CUVIDDECODECAPS) -> u32 =
                    nvcuvid_lib.get(b"cuvidGetDecoderCaps").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuvidGetDecoderCaps not found")
                    })?;

                // NVCUVID 関連
                let _: unsafe extern "C" fn(*mut sys::CUvideoctxlock, sys::CUcontext) -> u32 =
                    nvcuvid_lib.get(b"cuvidCtxLockCreate").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuvidCtxLockCreate not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUvideoctxlock) -> u32 =
                    nvcuvid_lib.get(b"cuvidCtxLockDestroy").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuvidCtxLockDestroy not found")
                    })?;

                let _: unsafe extern "C" fn(
                    *mut sys::CUvideoparser,
                    *mut sys::CUVIDPARSERPARAMS,
                ) -> u32 = nvcuvid_lib.get(b"cuvidCreateVideoParser").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidCreateVideoParser not found")
                })?;

                let _: unsafe extern "C" fn(sys::CUvideoparser) -> u32 =
                    nvcuvid_lib.get(b"cuvidDestroyVideoParser").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuvidDestroyVideoParser not found")
                    })?;

                let _: unsafe extern "C" fn(
                    sys::CUvideoparser,
                    *mut sys::CUVIDSOURCEDATAPACKET,
                ) -> u32 = nvcuvid_lib.get(b"cuvidParseVideoData").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidParseVideoData not found")
                })?;

                let _: unsafe extern "C" fn(
                    *mut sys::CUvideodecoder,
                    *mut sys::CUVIDDECODECREATEINFO,
                ) -> u32 = nvcuvid_lib.get(b"cuvidCreateDecoder").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidCreateDecoder not found")
                })?;

                let _: unsafe extern "C" fn(sys::CUvideodecoder) -> u32 =
                    nvcuvid_lib.get(b"cuvidDestroyDecoder").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuvidDestroyDecoder not found")
                    })?;

                let _: unsafe extern "C" fn(sys::CUvideodecoder, *mut sys::CUVIDPICPARAMS) -> u32 =
                    nvcuvid_lib.get(b"cuvidDecodePicture").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuvidDecodePicture not found")
                    })?;

                let _: unsafe extern "C" fn(
                    sys::CUvideodecoder,
                    i32,
                    *mut u64,
                    *mut u32,
                    *mut sys::CUVIDPROCPARAMS,
                ) -> u32 = nvcuvid_lib.get(b"cuvidMapVideoFrame64").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidMapVideoFrame64 not found")
                })?;

                let _: unsafe extern "C" fn(sys::CUvideodecoder, u64) -> u32 =
                    nvcuvid_lib.get(b"cuvidUnmapVideoFrame64").map_err(|_| {
                        Error::new_custom("CudaLibrary::load", "cuvidUnmapVideoFrame64 not found")
                    })?;

                // NVENC 関連
                let _: unsafe extern "C" fn(*mut sys::NV_ENCODE_API_FUNCTION_LIST) -> u32 =
                    nvenc_lib.get(b"NvEncodeAPICreateInstance").map_err(|_| {
                        Error::new_custom(
                            "CudaLibrary::load",
                            "NvEncodeAPICreateInstance not found",
                        )
                    })?;
            }

            Ok((cuda_lib, nvcuvid_lib, nvenc_lib))
        });

        let (cuda_lib, nvcuvid_lib, nvenc_lib) = LIBS.clone()?;

        Ok(Self {
            cuda_lib,
            nvcuvid_lib,
            nvenc_lib,
        })
    }

    /// エラーコードに対応する名前を取得する
    fn cu_get_error_name(&self, code: u32) -> Option<String> {
        unsafe {
            let f: unsafe extern "C" fn(u32, *mut *const u8) -> u32 =
                self.cuda_lib.get(b"cuGetErrorName").ok()?;

            let mut error_name: *const u8 = std::ptr::null();
            let status = f(code, &mut error_name);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                // NOTE: 無限再帰を避けるために、ここでは Error::check_cuda() は使わない
                return None;
            }
            if error_name.is_null() {
                // ここには来ないはずだけど保守的に NULL チェックを入れておく
                return None;
            }

            let error_str = std::ffi::CStr::from_ptr(error_name as *const std::ffi::c_char)
                .to_string_lossy()
                .into_owned();
            Some(error_str)
        }
    }

    /// エラーコードに対応するメッセージを取得する
    fn cu_get_error_string(&self, code: u32) -> Option<String> {
        unsafe {
            let f: unsafe extern "C" fn(u32, *mut *const u8) -> u32 =
                self.cuda_lib.get(b"cuGetErrorString").ok()?;

            let mut error_msg: *const u8 = std::ptr::null();
            let status = f(code, &mut error_msg);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                // NOTE: 無限再帰を避けるために、ここでは Error::check_cuda() は使わない
                return None;
            }
            if error_msg.is_null() {
                // ここには来ないはずだけど保守的に NULL チェックを入れておく
                return None;
            }

            let error_str = std::ffi::CStr::from_ptr(error_msg as *const std::ffi::c_char)
                .to_string_lossy()
                .into_owned();
            Some(error_str)
        }
    }

    /// CUDA コンテキストを作成する
    fn cu_ctx_create(
        &self,
        ctx: *mut sys::CUcontext,
        flags: u32,
        device: i32,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut sys::CUcontext, u32, i32) -> u32 = self
                .cuda_lib
                .get(b"cuCtxCreate_v2")
                .expect("cuCtxCreate_v2 should exist (checked in load())");
            let status = f(ctx, flags, device);
            Error::check_cuda(status, "cuCtxCreate_v2")
        }
    }

    /// CUDA コンテキストを破棄する
    fn cu_ctx_destroy(&self, ctx: sys::CUcontext) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUcontext) -> u32 = self
                .cuda_lib
                .get(b"cuCtxDestroy_v2")
                .expect("cuCtxDestroy_v2 should exist (checked in load())");
            let status = f(ctx);
            Error::check_cuda(status, "cuCtxDestroy_v2")
        }
    }

    /// CUDA コンテキストをスタックにプッシュする
    fn cu_ctx_push_current(&self, ctx: sys::CUcontext) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUcontext) -> u32 = self
                .cuda_lib
                .get(b"cuCtxPushCurrent_v2")
                .expect("cuCtxPushCurrent_v2 should exist (checked in load())");
            let status = f(ctx);
            Error::check_cuda(status, "cuCtxPushCurrent_v2")
        }
    }

    /// CUDA コンテキストをスタックからポップする
    fn cu_ctx_pop_current(&self, ctx: *mut sys::CUcontext) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut sys::CUcontext) -> u32 = self
                .cuda_lib
                .get(b"cuCtxPopCurrent_v2")
                .expect("cuCtxPopCurrent_v2 should exist (checked in load())");
            let status = f(ctx);
            Error::check_cuda(status, "cuCtxPopCurrent_v2")
        }
    }

    /// CUDA コンテキストを同期する
    fn cu_ctx_synchronize(&self) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn() -> u32 = self
                .cuda_lib
                .get(b"cuCtxSynchronize")
                .expect("cuCtxSynchronize should exist (checked in load())");
            let status = f();
            Error::check_cuda(status, "cuCtxSynchronize")
        }
    }

    /// デバイスメモリを割り当てる
    fn cu_mem_alloc(&self, dptr: *mut sys::CUdeviceptr, bytesize: usize) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut sys::CUdeviceptr, usize) -> u32 = self
                .cuda_lib
                .get(b"cuMemAlloc_v2")
                .expect("cuMemAlloc_v2 should exist (checked in load())");
            let status = f(dptr, bytesize);
            Error::check_cuda(status, "cuMemAlloc_v2")
        }
    }

    /// デバイスメモリを解放する
    fn cu_mem_free(&self, dptr: sys::CUdeviceptr) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUdeviceptr) -> u32 = self
                .cuda_lib
                .get(b"cuMemFree_v2")
                .expect("cuMemFree_v2 should exist (checked in load())");
            let status = f(dptr);
            Error::check_cuda(status, "cuMemFree_v2")
        }
    }

    /// ホストからデバイスへメモリをコピーする
    fn cu_memcpy_h_to_d(
        &self,
        dst_device: sys::CUdeviceptr,
        src_host: *const c_void,
        byte_count: usize,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUdeviceptr, *const c_void, usize) -> u32 = self
                .cuda_lib
                .get(b"cuMemcpyHtoD_v2")
                .expect("cuMemcpyHtoD_v2 should exist (checked in load())");
            let status = f(dst_device, src_host, byte_count);
            Error::check_cuda(status, "cuMemcpyHtoD_v2")
        }
    }

    /// デバイスからホストへメモリをコピーする
    fn cu_memcpy_d_to_h(
        &self,
        dst_host: *mut c_void,
        src_device: sys::CUdeviceptr,
        byte_count: usize,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, sys::CUdeviceptr, usize) -> u32 = self
                .cuda_lib
                .get(b"cuMemcpyDtoH_v2")
                .expect("cuMemcpyDtoH_v2 should exist (checked in load())");
            let status = f(dst_host, src_device, byte_count);
            Error::check_cuda(status, "cuMemcpyDtoH_v2")
        }
    }

    /// NvEncodeAPICreateInstance を呼び出す
    fn nvenc_create_api_instance(
        &self,
        function_list: *mut sys::NV_ENCODE_API_FUNCTION_LIST,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut sys::NV_ENCODE_API_FUNCTION_LIST) -> u32 = self
                .nvenc_lib
                .get(b"NvEncodeAPICreateInstance")
                .expect("NvEncodeAPICreateInstance should exist (checked in load())");
            let status = f(function_list);
            Error::check_nvenc(status, "NvEncodeAPICreateInstance")
        }
    }

    /// cuvidCtxLockCreate を呼び出す
    fn cuvid_ctx_lock_create(
        &self,
        lock: *mut sys::CUvideoctxlock,
        ctx: sys::CUcontext,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut sys::CUvideoctxlock, sys::CUcontext) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidCtxLockCreate")
                .expect("cuvidCtxLockCreate should exist (checked in load())");
            let status = f(lock, ctx);
            Error::check_cuda(status, "cuvidCtxLockCreate")
        }
    }

    /// cuvidCtxLockDestroy を呼び出す
    fn cuvid_ctx_lock_destroy(&self, lock: sys::CUvideoctxlock) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUvideoctxlock) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidCtxLockDestroy")
                .expect("cuvidCtxLockDestroy should exist (checked in load())");
            let status = f(lock);
            Error::check_cuda(status, "cuvidCtxLockDestroy")
        }
    }

    /// cuvidCreateVideoParser を呼び出す
    fn cuvid_create_video_parser(
        &self,
        parser: *mut sys::CUvideoparser,
        params: *mut sys::CUVIDPARSERPARAMS,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(
                *mut sys::CUvideoparser,
                *mut sys::CUVIDPARSERPARAMS,
            ) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidCreateVideoParser")
                .expect("cuvidCreateVideoParser should exist (checked in load())");
            let status = f(parser, params);
            Error::check_cuda(status, "cuvidCreateVideoParser")
        }
    }

    /// cuvidDestroyVideoParser を呼び出す
    fn cuvid_destroy_video_parser(&self, parser: sys::CUvideoparser) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUvideoparser) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidDestroyVideoParser")
                .expect("cuvidDestroyVideoParser should exist (checked in load())");
            let status = f(parser);
            Error::check_cuda(status, "cuvidDestroyVideoParser")
        }
    }

    /// cuvidParseVideoData を呼び出す
    fn cuvid_parse_video_data(
        &self,
        parser: sys::CUvideoparser,
        packet: *mut sys::CUVIDSOURCEDATAPACKET,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(
                sys::CUvideoparser,
                *mut sys::CUVIDSOURCEDATAPACKET,
            ) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidParseVideoData")
                .expect("cuvidParseVideoData should exist (checked in load())");
            let status = f(parser, packet);
            Error::check_cuda(status, "cuvidParseVideoData")
        }
    }

    /// cuvidCreateDecoder を呼び出す
    fn cuvid_create_decoder(
        &self,
        decoder: *mut sys::CUvideodecoder,
        create_info: *mut sys::CUVIDDECODECREATEINFO,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(
                *mut sys::CUvideodecoder,
                *mut sys::CUVIDDECODECREATEINFO,
            ) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidCreateDecoder")
                .expect("cuvidCreateDecoder should exist (checked in load())");
            let status = f(decoder, create_info);
            Error::check_cuda(status, "cuvidCreateDecoder")
        }
    }

    /// cuvidDestroyDecoder を呼び出す
    fn cuvid_destroy_decoder(&self, decoder: sys::CUvideodecoder) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUvideodecoder) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidDestroyDecoder")
                .expect("cuvidDestroyDecoder should exist (checked in load())");
            let status = f(decoder);
            Error::check_cuda(status, "cuvidDestroyDecoder")
        }
    }

    /// cuvidDecodePicture を呼び出す
    fn cuvid_decode_picture(
        &self,
        decoder: sys::CUvideodecoder,
        pic_params: *mut sys::CUVIDPICPARAMS,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUvideodecoder, *mut sys::CUVIDPICPARAMS) -> u32 =
                self.nvcuvid_lib
                    .get(b"cuvidDecodePicture")
                    .expect("cuvidDecodePicture should exist (checked in load())");
            let status = f(decoder, pic_params);
            Error::check_cuda(status, "cuvidDecodePicture")
        }
    }

    /// cuvidMapVideoFrame64 を呼び出す
    fn cuvid_map_video_frame(
        &self,
        decoder: sys::CUvideodecoder,
        picture_index: i32,
        device_ptr: *mut u64,
        pitch: *mut u32,
        proc_params: *mut sys::CUVIDPROCPARAMS,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(
                sys::CUvideodecoder,
                i32,
                *mut u64,
                *mut u32,
                *mut sys::CUVIDPROCPARAMS,
            ) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidMapVideoFrame64")
                .expect("cuvidMapVideoFrame64 should exist (checked in load())");
            let status = f(decoder, picture_index, device_ptr, pitch, proc_params);
            Error::check_cuda(status, "cuvidMapVideoFrame64")
        }
    }

    /// cuvidUnmapVideoFrame64 を呼び出す
    fn cuvid_unmap_video_frame(
        &self,
        decoder: sys::CUvideodecoder,
        device_ptr: u64,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUvideodecoder, u64) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidUnmapVideoFrame64")
                .expect("cuvidUnmapVideoFrame64 should exist (checked in load())");
            let status = f(decoder, device_ptr);
            Error::check_cuda(status, "cuvidUnmapVideoFrame64")
        }
    }

    /// CUDA context を push して、クロージャを実行し、自動的に pop する
    ///
    /// クロージャが panic しても pop は必ず実行される（CUDA コンテキストスタックの整合性を保つため）
    fn with_context<F, R>(&self, ctx: sys::CUcontext, f: F) -> Result<R, Error>
    where
        F: FnOnce() -> Result<R, Error>,
    {
        self.cu_ctx_push_current(ctx)?;

        // panic 時にも必ず pop するためのガード（panic 時は戻り値を検査できないので握り潰す）
        let pop_guard = crate::ReleaseGuard::new(|| {
            let mut popped_ctx = std::ptr::null_mut();
            let _ = self.cu_ctx_pop_current(&mut popped_ctx);
        });

        let result = f();

        // 正常パスではガードをキャンセルし、明示的に pop してエラーを検査する
        pop_guard.cancel();
        let mut popped_ctx = std::ptr::null_mut();
        self.cu_ctx_pop_current(&mut popped_ctx)?;

        result
    }

    /// CUDA デバイスの数を取得する
    fn cu_device_get_count(&self) -> Result<i32, Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut c_int) -> u32 = self
                .cuda_lib
                .get(b"cuDeviceGetCount")
                .expect("cuDeviceGetCount should exist (checked in load())");
            let mut count: c_int = 0;
            let status = f(&mut count);
            Error::check_cuda(status, "cuDeviceGetCount")?;
            Ok(count)
        }
    }

    /// CUDA デバイスハンドルを取得する
    fn cu_device_get(&self, ordinal: i32) -> Result<i32, Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut c_int, c_int) -> u32 = self
                .cuda_lib
                .get(b"cuDeviceGet")
                .expect("cuDeviceGet should exist (checked in load())");
            let mut device: c_int = 0;
            let status = f(&mut device, ordinal);
            Error::check_cuda(status, "cuDeviceGet")?;
            Ok(device)
        }
    }

    /// CUDA デバイス名を取得する
    fn cu_device_get_name(&self, device: i32) -> Result<String, Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut c_char, c_int, c_int) -> u32 = self
                .cuda_lib
                .get(b"cuDeviceGetName")
                .expect("cuDeviceGetName should exist (checked in load())");
            let mut name_buf = [0u8; 256];
            let status = f(name_buf.as_mut_ptr() as *mut c_char, 256, device);
            Error::check_cuda(status, "cuDeviceGetName")?;
            let name = std::ffi::CStr::from_ptr(name_buf.as_ptr() as *const c_char)
                .to_string_lossy()
                .into_owned();
            Ok(name)
        }
    }

    /// ピッチ付きデバイスメモリを割り当てる
    fn cu_mem_alloc_pitch(
        &self,
        dptr: *mut sys::CUdeviceptr,
        pitch: *mut usize,
        width_in_bytes: usize,
        height: usize,
        element_size_bytes: u32,
    ) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(
                *mut sys::CUdeviceptr,
                *mut usize,
                usize,
                usize,
                c_uint,
            ) -> u32 = self
                .cuda_lib
                .get(b"cuMemAllocPitch_v2")
                .expect("cuMemAllocPitch_v2 should exist (checked in load())");
            let status = f(dptr, pitch, width_in_bytes, height, element_size_bytes);
            Error::check_cuda(status, "cuMemAllocPitch_v2")
        }
    }

    /// 2D メモリコピーを実行する
    fn cu_memcpy_2d(&self, copy_params: *const sys::CUDA_MEMCPY2D) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*const sys::CUDA_MEMCPY2D) -> u32 = self
                .cuda_lib
                .get(b"cuMemcpy2D_v2")
                .expect("cuMemcpy2D_v2 should exist (checked in load())");
            let status = f(copy_params);
            Error::check_cuda(status, "cuMemcpy2D_v2")
        }
    }

    /// CUDA ストリームを作成する
    fn cu_stream_create(&self, stream: *mut sys::CUstream, flags: u32) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut sys::CUstream, c_uint) -> u32 = self
                .cuda_lib
                .get(b"cuStreamCreate")
                .expect("cuStreamCreate should exist (checked in load())");
            let status = f(stream, flags);
            Error::check_cuda(status, "cuStreamCreate")
        }
    }

    /// CUDA ストリームを同期する
    fn cu_stream_synchronize(&self, stream: sys::CUstream) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUstream) -> u32 = self
                .cuda_lib
                .get(b"cuStreamSynchronize")
                .expect("cuStreamSynchronize should exist (checked in load())");
            let status = f(stream);
            Error::check_cuda(status, "cuStreamSynchronize")
        }
    }

    /// CUDA ストリームを破棄する
    fn cu_stream_destroy(&self, stream: sys::CUstream) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(sys::CUstream) -> u32 = self
                .cuda_lib
                .get(b"cuStreamDestroy_v2")
                .expect("cuStreamDestroy_v2 should exist (checked in load())");
            let status = f(stream);
            Error::check_cuda(status, "cuStreamDestroy_v2")
        }
    }

    /// デコーダのケーパビリティをクエリする
    fn cuvid_get_decoder_caps(&self, caps: *mut sys::CUVIDDECODECAPS) -> Result<(), Error> {
        unsafe {
            let f: unsafe extern "C" fn(*mut sys::CUVIDDECODECAPS) -> u32 = self
                .nvcuvid_lib
                .get(b"cuvidGetDecoderCaps")
                .expect("cuvidGetDecoderCaps should exist (checked in load())");
            let status = f(caps);
            Error::check_cuda(status, "cuvidGetDecoderCaps")
        }
    }
}

/// CUDA ライブラリがロード可能かチェックする
///
/// NOTE:
/// この関数がチェックするのは、あくまでも .so などが読み込めるかどうか、までで
/// その環境で実際に CUDA が利用可能かどうかまでは確認していない
pub fn is_cuda_library_available() -> bool {
    dl::DynLib::open(Path::new("libcuda.so.1")).is_ok()
}

/// CUDA デバイスの数を取得する
pub fn device_count() -> Result<i32, Error> {
    let lib = CudaLibrary::load()?;
    lib.cu_device_get_count()
}

/// CUDA デバイスの名前を取得する
pub fn device_name(device_id: i32) -> Result<String, Error> {
    let lib = CudaLibrary::load()?;
    let device = lib.cu_device_get(device_id)?;
    lib.cu_device_get_name(device)
}

/// メモリタイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryType {
    /// ホストメモリ
    Host,
    /// デバイスメモリ
    Device,
}

/// 2D メモリコピーのパラメータ
#[derive(Debug, Clone)]
pub struct Memcpy2DParams {
    /// コピー元のメモリタイプ
    pub src_memory_type: MemoryType,
    /// コピー元のホストポインタ
    pub src_host: *const c_void,
    /// コピー元のデバイスポインタ
    pub src_device: u64,
    /// コピー元のピッチ (バイト単位)
    pub src_pitch: usize,
    /// コピー先のメモリタイプ
    pub dst_memory_type: MemoryType,
    /// コピー先のホストポインタ
    pub dst_host: *mut c_void,
    /// コピー先のデバイスポインタ
    pub dst_device: u64,
    /// コピー先のピッチ (バイト単位)
    pub dst_pitch: usize,
    /// コピーする幅 (バイト単位)
    pub width_in_bytes: usize,
    /// コピーする高さ (行数)
    pub height: usize,
}

/// 2D メモリコピーを実行する
///
/// ピッチの異なるメモリ領域間でのコピーに使用する
pub fn memcpy_2d(params: &Memcpy2DParams) -> Result<(), Error> {
    let lib = CudaLibrary::load()?;

    let src_type = match params.src_memory_type {
        MemoryType::Host => sys::CUmemorytype_enum_CU_MEMORYTYPE_HOST,
        MemoryType::Device => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
    };
    let dst_type = match params.dst_memory_type {
        MemoryType::Host => sys::CUmemorytype_enum_CU_MEMORYTYPE_HOST,
        MemoryType::Device => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
    };

    unsafe {
        let mut copy: sys::CUDA_MEMCPY2D = std::mem::zeroed();
        copy.srcMemoryType = src_type;
        copy.srcHost = params.src_host;
        copy.srcDevice = params.src_device;
        copy.srcPitch = params.src_pitch;
        copy.dstMemoryType = dst_type;
        copy.dstHost = params.dst_host;
        copy.dstDevice = params.dst_device;
        copy.dstPitch = params.dst_pitch;
        copy.WidthInBytes = params.width_in_bytes;
        copy.Height = params.height;

        lib.cu_memcpy_2d(&copy)
    }
}

/// ピッチ付きデバイスメモリを割り当てる
///
/// GPU のアライメント要件に合わせたピッチ（ストライド）でメモリを割り当てる。
/// 戻り値はデバイスポインタとピッチのタプル。
pub fn mem_alloc_pitch(
    width_in_bytes: usize,
    height: usize,
    element_size_bytes: u32,
) -> Result<(u64, usize), Error> {
    let lib = CudaLibrary::load()?;
    let mut dptr: sys::CUdeviceptr = 0;
    let mut pitch: usize = 0;
    lib.cu_mem_alloc_pitch(
        &mut dptr,
        &mut pitch,
        width_in_bytes,
        height,
        element_size_bytes,
    )?;
    Ok((dptr, pitch))
}

/// CUDA ストリーム
///
/// 非同期 GPU 操作のキューイングに使用する
pub struct CudaStream {
    lib: CudaLibrary,
    stream: sys::CUstream,
}

impl CudaStream {
    /// 新しい CUDA ストリームを作成する
    pub fn new() -> Result<Self, Error> {
        let lib = CudaLibrary::load()?;
        let mut stream = std::ptr::null_mut();
        let flags = 0; // デフォルトフラグ
        lib.cu_stream_create(&mut stream, flags)?;
        Ok(Self { lib, stream })
    }

    /// ストリーム内の全ての操作が完了するまで待機する
    pub fn synchronize(&self) -> Result<(), Error> {
        self.lib.cu_stream_synchronize(self.stream)
    }

    /// 内部の CUstream ハンドルを取得する
    pub fn as_raw(&self) -> sys::CUstream {
        self.stream
    }
}

impl Drop for CudaStream {
    fn drop(&mut self) {
        if !self.stream.is_null() {
            let _ = self.lib.cu_stream_destroy(self.stream);
        }
    }
}

impl std::fmt::Debug for CudaStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CudaStream")
            .field("stream", &format_args!("{:p}", self.stream))
            .finish()
    }
}

unsafe impl Send for CudaStream {}

/// エラー時にリソースを確実に解放するための構造体
struct ReleaseGuard<F: FnOnce()> {
    cleanup: Option<F>,
}

impl<F: FnOnce()> ReleaseGuard<F> {
    /// 新しい ReleaseGuard を作成する
    fn new(cleanup: F) -> Self {
        Self {
            cleanup: Some(cleanup),
        }
    }

    /// クリーンアップ処理をキャンセルする（リソースの所有権が移転した場合などに使用）
    fn cancel(mut self) {
        self.cleanup = None;
    }
}

impl<F: FnOnce()> Drop for ReleaseGuard<F> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}
