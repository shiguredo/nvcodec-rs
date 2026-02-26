// cuda.h スタブヘッダ
//
// NVIDIA Video Codec SDK のヘッダ (cuviddec.h, nvcuvid.h) が必要とする
// 最小限の CUDA Driver API 型定義のみを提供する。
//
// CUDA Toolkit がインストールされていない環境 (macOS など) で
// バインディング生成を可能にするためのもの。
//
// 参照元: NVIDIA Video Codec SDK 13.0.19

#ifndef __cuda_cuda_h__
#define __cuda_cuda_h__

#include <stddef.h>

#define CUDA_VERSION 13000

#if defined(_WIN32) || defined(__CYGWIN__)
#define CUDAAPI __stdcall
#else
#define CUDAAPI
#endif

typedef enum cudaError_enum {
    CUDA_SUCCESS = 0,
} CUresult;

typedef int CUdevice;
typedef struct CUctx_st *CUcontext;
typedef struct CUstream_st *CUstream;

#if defined(__x86_64) || defined(AMD64) || defined(_M_AMD64) || defined(__aarch64__) || defined(__ppc64__) || defined(__LP64__)
typedef unsigned long long CUdeviceptr;
#else
typedef unsigned int CUdeviceptr;
#endif

typedef struct CUarray_st *CUarray;

typedef enum CUmemorytype_enum {
    CU_MEMORYTYPE_HOST = 0x01,
    CU_MEMORYTYPE_DEVICE = 0x02,
    CU_MEMORYTYPE_ARRAY = 0x03,
    CU_MEMORYTYPE_UNIFIED = 0x04,
} CUmemorytype;

typedef struct CUDA_MEMCPY2D_st {
    size_t srcXInBytes;
    size_t srcY;
    CUmemorytype srcMemoryType;
    const void *srcHost;
    CUdeviceptr srcDevice;
    CUarray srcArray;
    size_t srcPitch;
    size_t dstXInBytes;
    size_t dstY;
    CUmemorytype dstMemoryType;
    void *dstHost;
    CUdeviceptr dstDevice;
    CUarray dstArray;
    size_t dstPitch;
    size_t WidthInBytes;
    size_t Height;
} CUDA_MEMCPY2D_v2;
typedef CUDA_MEMCPY2D_v2 CUDA_MEMCPY2D;

#endif /* __cuda_cuda_h__ */
