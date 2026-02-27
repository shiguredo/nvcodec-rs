use std::path::{Path, PathBuf};

const DEFAULT_CUDA_INCLUDE_PATH: &str = "/usr/local/cuda/include/";
const CUDA_INCLUDE_PATH_ENV_KEY: &str = "CUDA_INCLUDE_PATH";

fn main() {
    // Cargo.toml か build.rs か third_party のヘッダファイルが更新されたら、バインディングファイルを再生成する
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=third_party/nvcodec/include/");
    println!("cargo::rerun-if-changed=third_party/cuda/include/");
    println!("cargo::rerun-if-env-changed=CUDA_INCLUDE_PATH");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let output_bindings_path = out_dir.join("bindings.rs");
    let output_metadata_path = out_dir.join("metadata.rs");

    // 各種メタデータを書き込む
    let version = get_version();
    std::fs::write(
        output_metadata_path,
        format!("pub const BUILD_METADATA_VERSION: &str={:?};\n", version),
    )
    .expect("failed to write metadata file");

    // third_party にあるヘッダファイルのパス
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("infallible"));
    let third_party_header_dir = manifest_dir.join("third_party/nvcodec/include");

    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは外部ファイルのダウンロードができないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(output_bindings_path, generate_docs_rs_stub()).expect("write file error");
        return;
    }

    // third_party のヘッダファイルが存在することを確認
    if !third_party_header_dir.exists() {
        panic!(
            "Third party nvcodec headers not found at {:?}. Please ensure the headers are placed in third_party/nvcodec/include/",
            third_party_header_dir
        );
    }

    let nvenc_header = third_party_header_dir.join("nvEncodeAPI.h");
    let cuvid_header = third_party_header_dir.join("cuviddec.h");
    let nvcuvid_header = third_party_header_dir.join("nvcuvid.h");

    if !nvenc_header.exists() {
        panic!("nvEncodeAPI.h not found at {:?}", nvenc_header);
    }
    if !cuvid_header.exists() {
        panic!("cuviddec.h not found at {:?}", cuvid_header);
    }
    if !nvcuvid_header.exists() {
        panic!("nvcuvid.h not found at {:?}", nvcuvid_header);
    }

    // CUDA インクルードパスを取得
    let cuda_include_path = resolve_cuda_include_path(&manifest_dir);

    // バインディングを生成する
    let bindings = bindgen::Builder::default()
        .header(nvenc_header.display().to_string())
        .header(cuvid_header.display().to_string())
        .header(nvcuvid_header.display().to_string())
        .clang_arg(format!("-I{}", cuda_include_path.display()))
        .generate_comments(false)
        .derive_debug(false)
        .derive_default(false)
        .parse_callbacks(Box::new(CustomCallbacks))
        // GUID は bindgen で正しく生成されないため、ここではブラックリストに登録して、後で手動で定義する
        .blocklist_item("NV_ENC_CODEC_H264_GUID")
        .blocklist_item("NV_ENC_CODEC_HEVC_GUID")
        .blocklist_item("NV_ENC_CODEC_AV1_GUID")
        .blocklist_item("NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_BASELINE_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_MAIN_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_10_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_422_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_HIGH_444_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_STEREO_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID")
        .blocklist_item("NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID")
        .blocklist_item("NV_ENC_HEVC_PROFILE_MAIN_GUID")
        .blocklist_item("NV_ENC_HEVC_PROFILE_MAIN10_GUID")
        .blocklist_item("NV_ENC_HEVC_PROFILE_FREXT_GUID")
        .blocklist_item("NV_ENC_AV1_PROFILE_MAIN_GUID")
        .blocklist_item("NV_ENC_PRESET_P1_GUID")
        .blocklist_item("NV_ENC_PRESET_P2_GUID")
        .blocklist_item("NV_ENC_PRESET_P3_GUID")
        .blocklist_item("NV_ENC_PRESET_P4_GUID")
        .blocklist_item("NV_ENC_PRESET_P5_GUID")
        .blocklist_item("NV_ENC_PRESET_P6_GUID")
        .blocklist_item("NV_ENC_PRESET_P7_GUID")
        .generate()
        .expect("failed to generate bindings");

    // バージョン定数と GUID 定義を追加する
    let additional_definitions = r#"

// nvEncodeAPI.h のバージョン定数
// これらは C のマクロなので、bindgen は自動的に生成しない
const NVENCAPI_STRUCT_VERSION_BASE: u32 = 0x7 << 28;

pub const NV_ENCODE_API_FUNCTION_LIST_VER: u32 = NVENCAPI_VERSION | (2 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER: u32 = NVENCAPI_VERSION | (1 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_PRESET_CONFIG_VER: u32 = NVENCAPI_VERSION | (5 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_CONFIG_VER: u32 = NVENCAPI_VERSION | (9 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_INITIALIZE_PARAMS_VER: u32 = NVENCAPI_VERSION | (7 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_CREATE_BITSTREAM_BUFFER_VER: u32 = NVENCAPI_VERSION | (1 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_PIC_PARAMS_VER: u32 = NVENCAPI_VERSION | (7 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_LOCK_BITSTREAM_VER: u32 = NVENCAPI_VERSION | (2 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);
pub const NV_ENC_REGISTER_RESOURCE_VER: u32 = NVENCAPI_VERSION | (5 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_MAP_INPUT_RESOURCE_VER: u32 = NVENCAPI_VERSION | (4 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_SEQUENCE_PARAM_PAYLOAD_VER: u32 = NVENCAPI_VERSION | (1 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_CAPS_PARAM_VER: u32 = NVENCAPI_VERSION | (1 << 16) | NVENCAPI_STRUCT_VERSION_BASE;
pub const NV_ENC_RECONFIGURE_PARAMS_VER: u32 = NVENCAPI_VERSION | (2 << 16) | NVENCAPI_STRUCT_VERSION_BASE | (1 << 31);

// ピクチャーフラグ (NV_ENC_PIC_FLAGS)
pub const NV_ENC_PIC_FLAG_FORCEINTRA: u32 = 0x1;
pub const NV_ENC_PIC_FLAG_FORCEIDR: u32 = 0x2;
pub const NV_ENC_PIC_FLAG_OUTPUT_SPSPPS: u32 = 0x4;
pub const NV_ENC_PIC_FLAG_EOS: u32 = 0x8;

// crate で使用される NVENC GUID 定数
// これらの GUID はリンクの問題を避けるために extern static ではなく定数として定義されている。

// コーデック GUID: NV_ENC_CODEC_H264_GUID
// {6BC82762-4E63-4ca4-AA85-1E50F321F6BF}
pub const NV_ENC_CODEC_H264_GUID: GUID = GUID {
    Data1: 0x6bc82762,
    Data2: 0x4e63,
    Data3: 0x4ca4,
    Data4: [0xaa, 0x85, 0x1e, 0x50, 0xf3, 0x21, 0xf6, 0xbf],
};

// コーデック GUID: NV_ENC_CODEC_HEVC_GUID
// {790CDC88-4522-4d7b-9425-BDA9975F7603}
pub const NV_ENC_CODEC_HEVC_GUID: GUID = GUID {
    Data1: 0x790cdc88,
    Data2: 0x4522,
    Data3: 0x4d7b,
    Data4: [0x94, 0x25, 0xbd, 0xa9, 0x97, 0x5f, 0x76, 0x03],
};

// コーデック GUID: NV_ENC_CODEC_AV1_GUID
// {0A352289-0AA7-4759-862D-5D15CD16D254}
pub const NV_ENC_CODEC_AV1_GUID: GUID = GUID {
    Data1: 0x0a352289,
    Data2: 0x0aa7,
    Data3: 0x4759,
    Data4: [0x86, 0x2d, 0x5d, 0x15, 0xcd, 0x16, 0xd2, 0x54],
};

// プロファイル GUID: NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID
// {BFD6F8E7-233C-4341-8B3E-4818523803F4}
pub const NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID: GUID = GUID {
    Data1: 0xbfd6f8e7,
    Data2: 0x233c,
    Data3: 0x4341,
    Data4: [0x8b, 0x3e, 0x48, 0x18, 0x52, 0x38, 0x03, 0xf4],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_BASELINE_GUID
// {0727BCAA-78C4-4c83-8C2F-EF3DFF267C6A}
pub const NV_ENC_H264_PROFILE_BASELINE_GUID: GUID = GUID {
    Data1: 0x0727bcaa,
    Data2: 0x78c4,
    Data3: 0x4c83,
    Data4: [0x8c, 0x2f, 0xef, 0x3d, 0xff, 0x26, 0x7c, 0x6a],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_MAIN_GUID
// {60B5C1D4-67FE-4790-94D5-C4726D7B6E6D}
pub const NV_ENC_H264_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0x60b5c1d4,
    Data2: 0x67fe,
    Data3: 0x4790,
    Data4: [0x94, 0xd5, 0xc4, 0x72, 0x6d, 0x7b, 0x6e, 0x6d],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_GUID
// {E7CBC309-4F7A-4b89-AF2A-D537C92BE310}
pub const NV_ENC_H264_PROFILE_HIGH_GUID: GUID = GUID {
    Data1: 0xe7cbc309,
    Data2: 0x4f7a,
    Data3: 0x4b89,
    Data4: [0xaf, 0x2a, 0xd5, 0x37, 0xc9, 0x2b, 0xe3, 0x10],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_10_GUID
// {8F0C337E-186C-48E9-A69D-7A8334089758}
pub const NV_ENC_H264_PROFILE_HIGH_10_GUID: GUID = GUID {
    Data1: 0x8f0c337e,
    Data2: 0x186c,
    Data3: 0x48e9,
    Data4: [0xa6, 0x9d, 0x7a, 0x83, 0x34, 0x08, 0x97, 0x58],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_422_GUID
// {FF3242E9-613C-4295-A1E8-2A7FE94D8133}
pub const NV_ENC_H264_PROFILE_HIGH_422_GUID: GUID = GUID {
    Data1: 0xff3242e9,
    Data2: 0x613c,
    Data3: 0x4295,
    Data4: [0xa1, 0xe8, 0x2a, 0x7f, 0xe9, 0x4d, 0x81, 0x33],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_HIGH_444_GUID
// {7AC663CB-A598-4960-B844-339B261A7D52}
pub const NV_ENC_H264_PROFILE_HIGH_444_GUID: GUID = GUID {
    Data1: 0x7ac663cb,
    Data2: 0xa598,
    Data3: 0x4960,
    Data4: [0xb8, 0x44, 0x33, 0x9b, 0x26, 0x1a, 0x7d, 0x52],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_STEREO_GUID
// {40847BF5-33F7-4601-9084-E8FE3C1DB8B7}
pub const NV_ENC_H264_PROFILE_STEREO_GUID: GUID = GUID {
    Data1: 0x40847bf5,
    Data2: 0x33f7,
    Data3: 0x4601,
    Data4: [0x90, 0x84, 0xe8, 0xfe, 0x3c, 0x1d, 0xb8, 0xb7],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID
// {B405AFAC-F32B-417B-89C4-9ABEED3E5978}
pub const NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID: GUID = GUID {
    Data1: 0xb405afac,
    Data2: 0xf32b,
    Data3: 0x417b,
    Data4: [0x89, 0xc4, 0x9a, 0xbe, 0xed, 0x3e, 0x59, 0x78],
};

// プロファイル GUID: NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID
// {AEC1BD87-E85B-48f2-84C3-98BCA6285072}
pub const NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID: GUID = GUID {
    Data1: 0xaec1bd87,
    Data2: 0xe85b,
    Data3: 0x48f2,
    Data4: [0x84, 0xc3, 0x98, 0xbc, 0xa6, 0x28, 0x50, 0x72],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_MAIN_GUID
// {B514C39A-B55B-40fa-878F-F1253B4DFDEC}
pub const NV_ENC_HEVC_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0xb514c39a,
    Data2: 0xb55b,
    Data3: 0x40fa,
    Data4: [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_MAIN10_GUID
// {fa4d2b6c-3a5b-411a-8018-0a3f5e3c9be5}
pub const NV_ENC_HEVC_PROFILE_MAIN10_GUID: GUID = GUID {
    Data1: 0xfa4d2b6c,
    Data2: 0x3a5b,
    Data3: 0x411a,
    Data4: [0x80, 0x18, 0x0a, 0x3f, 0x5e, 0x3c, 0x9b, 0xe5],
};

// プロファイル GUID: NV_ENC_HEVC_PROFILE_FREXT_GUID
// {51ec32b5-1b4c-453c-9cbd-b616bd621341}
pub const NV_ENC_HEVC_PROFILE_FREXT_GUID: GUID = GUID {
    Data1: 0x51ec32b5,
    Data2: 0x1b4c,
    Data3: 0x453c,
    Data4: [0x9c, 0xbd, 0xb6, 0x16, 0xbd, 0x62, 0x13, 0x41],
};

// プロファイル GUID: NV_ENC_AV1_PROFILE_MAIN_GUID
// {5f2a39f5-f14e-4f95-9a9e-b76d568fcf97}
pub const NV_ENC_AV1_PROFILE_MAIN_GUID: GUID = GUID {
    Data1: 0x5f2a39f5,
    Data2: 0xf14e,
    Data3: 0x4f95,
    Data4: [0x9a, 0x9e, 0xb7, 0x6d, 0x56, 0x8f, 0xcf, 0x97],
};

// プリセット GUID: NV_ENC_PRESET_P1_GUID
// {FC0A8D3E-45F8-4CF8-80C7-298871590EBF}
pub const NV_ENC_PRESET_P1_GUID: GUID = GUID {
    Data1: 0xfc0a8d3e,
    Data2: 0x45f8,
    Data3: 0x4cf8,
    Data4: [0x80, 0xc7, 0x29, 0x88, 0x71, 0x59, 0x0e, 0xbf],
};

// プリセット GUID: NV_ENC_PRESET_P2_GUID
// {F581CFB8-88D6-4381-93F0-DF13F9C27DAB}
pub const NV_ENC_PRESET_P2_GUID: GUID = GUID {
    Data1: 0xf581cfb8,
    Data2: 0x88d6,
    Data3: 0x4381,
    Data4: [0x93, 0xf0, 0xdf, 0x13, 0xf9, 0xc2, 0x7d, 0xab],
};

// プリセット GUID: NV_ENC_PRESET_P3_GUID
// {36850110-3A07-441F-94D5-3670631F91F6}
pub const NV_ENC_PRESET_P3_GUID: GUID = GUID {
    Data1: 0x36850110,
    Data2: 0x3a07,
    Data3: 0x441f,
    Data4: [0x94, 0xd5, 0x36, 0x70, 0x63, 0x1f, 0x91, 0xf6],
};

// プリセット GUID: NV_ENC_PRESET_P4_GUID
// {90A7B826-DF06-4862-B9D2-CD6D73A08681}
pub const NV_ENC_PRESET_P4_GUID: GUID = GUID {
    Data1: 0x90a7b826,
    Data2: 0xdf06,
    Data3: 0x4862,
    Data4: [0xb9, 0xd2, 0xcd, 0x6d, 0x73, 0xa0, 0x86, 0x81],
};

// プリセット GUID: NV_ENC_PRESET_P5_GUID
// {21C6E6B4-297A-4CBA-998F-B6CBDE72ADE3}
pub const NV_ENC_PRESET_P5_GUID: GUID = GUID {
    Data1: 0x21c6e6b4,
    Data2: 0x297a,
    Data3: 0x4cba,
    Data4: [0x99, 0x8f, 0xb6, 0xcb, 0xde, 0x72, 0xad, 0xe3],
};

// プリセット GUID: NV_ENC_PRESET_P6_GUID
// {8E75C279-6299-4AB6-8302-0B215A335CF5}
pub const NV_ENC_PRESET_P6_GUID: GUID = GUID {
    Data1: 0x8e75c279,
    Data2: 0x6299,
    Data3: 0x4ab6,
    Data4: [0x83, 0x02, 0x0b, 0x21, 0x5a, 0x33, 0x5c, 0xf5],
};

// プリセット GUID: NV_ENC_PRESET_P7_GUID
// {84848C12-6F71-4C13-931B-53E283F57974}
pub const NV_ENC_PRESET_P7_GUID: GUID = GUID {
    Data1: 0x84848c12,
    Data2: 0x6f71,
    Data3: 0x4c13,
    Data4: [0x93, 0x1b, 0x53, 0xe2, 0x83, 0xf5, 0x79, 0x74],
};
"#;

    // 追加の定義を付加してバインディングを書き込む
    std::fs::write(
        &output_bindings_path,
        format!("{bindings}\n{additional_definitions}"),
    )
    .expect("failed to write bindings");
}

// CUDA インクルードパスを解決する
//
// 優先順位:
// 1. CUDA_INCLUDE_PATH 環境変数
// 2. デフォルトパス (/usr/local/cuda/include/)
// 3. スタブヘッダ (third_party/cuda/include/)
fn resolve_cuda_include_path(manifest_dir: &Path) -> PathBuf {
    // 環境変数が設定されている場合はそれを使用する
    if let Ok(env_path) = std::env::var(CUDA_INCLUDE_PATH_ENV_KEY) {
        let path = PathBuf::from(&env_path);
        if path.join("cuda.h").exists() {
            return path;
        }
        panic!(
            "CUDA_INCLUDE_PATH is set to {:?} but cuda.h was not found there",
            env_path,
        );
    }

    // デフォルトパスに cuda.h が存在する場合はそれを使用する
    let default_path = PathBuf::from(DEFAULT_CUDA_INCLUDE_PATH);
    if default_path.join("cuda.h").exists() {
        return default_path;
    }

    // スタブヘッダにフォールバックする
    let stub_path = manifest_dir.join("third_party/cuda/include");
    if stub_path.join("cuda.h").exists() {
        println!(
            "cargo::warning=CUDA Toolkit not found. Using stub cuda.h from {:?}",
            stub_path,
        );
        return stub_path;
    }

    panic!(
        r#"cuda.h not found.

Searched locations:
1. Default: {DEFAULT_CUDA_INCLUDE_PATH}
2. Stub: {:?}

To resolve this issue:
1. Install CUDA Toolkit, or
2. Set CUDA_INCLUDE_PATH environment variable, or
3. Ensure third_party/cuda/include/cuda.h exists
"#,
        stub_path,
    );
}

// Cargo.toml から依存ライブラリのバージョンを取得する
fn get_version() -> String {
    let cargo_toml =
        shiguredo_toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some(version) = shiguredo_toml::Value::Table(cargo_toml)
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get("nvcodec"))
        .and_then(|v| v.get("version"))
        .and_then(|s| s.as_str())
    {
        version.to_string()
    } else {
        panic!(
            "Cargo.toml does not contain a valid [package.metadata.external-dependencies.nvcodec] version"
        );
    }
}

#[derive(Debug)]
struct CustomCallbacks;

impl bindgen::callbacks::ParseCallbacks for CustomCallbacks {
    fn add_derives(&self, info: &bindgen::callbacks::DeriveInfo<'_>) -> Vec<String> {
        // "_GUID" に各種トレイトを導出
        if info.name == "_GUID" {
            vec![
                "Debug".to_string(),
                "PartialEq".to_string(),
                "Eq".to_string(),
            ]
        } else {
            vec![]
        }
    }
}

/// docs.rs 向けのダミーバインディングを生成する
///
/// docs.rs ではネイティブライブラリのダウンロードやリンクができないため、
/// コンパイルに必要な最低限の型定義だけをダミーで出力する。
fn generate_docs_rs_stub() -> String {
    r#"
// --- 基本型 ---
pub type CUdeviceptr = u64;
pub type CUcontext = *mut std::ffi::c_void;
pub type CUstream = *mut std::ffi::c_void;
pub type CUvideoparser = *mut std::ffi::c_void;
pub type CUvideodecoder = *mut std::ffi::c_void;
pub type CUvideoctxlock = *mut std::ffi::c_void;
pub type cudaVideoCodec = u32;
pub type NV_ENC_BUFFER_FORMAT = u32;
pub type NV_ENC_INPUT_PTR = *mut std::ffi::c_void;
pub type NV_ENC_OUTPUT_PTR = *mut std::ffi::c_void;
pub type NV_ENC_REGISTERED_PTR = *mut std::ffi::c_void;
pub type NV_ENC_PIC_TYPE = u32;
pub type NV_ENC_TUNING_INFO = u32;
pub type NV_ENC_PARAMS_RC_MODE = u32;
pub type CUresult = u32;
pub type CUmemorytype = u32;

// --- GUID ---
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct GUID {
    pub Data1: u32,
    pub Data2: u16,
    pub Data3: u16,
    pub Data4: [u8; 8],
}

// --- CUDA エラー ---
pub const cudaError_enum_CUDA_SUCCESS: u32 = 0;

// --- メモリタイプ ---
pub const CUmemorytype_enum_CU_MEMORYTYPE_HOST: CUmemorytype = 1;
pub const CUmemorytype_enum_CU_MEMORYTYPE_DEVICE: CUmemorytype = 2;

// --- CUDA_MEMCPY2D ---
#[repr(C)]
pub struct CUDA_MEMCPY2D {
    pub srcXInBytes: usize,
    pub srcY: usize,
    pub srcMemoryType: CUmemorytype,
    pub srcHost: *const std::ffi::c_void,
    pub srcDevice: CUdeviceptr,
    pub srcArray: *mut std::ffi::c_void,
    pub srcPitch: usize,
    pub dstXInBytes: usize,
    pub dstY: usize,
    pub dstMemoryType: CUmemorytype,
    pub dstHost: *mut std::ffi::c_void,
    pub dstDevice: CUdeviceptr,
    pub dstArray: *mut std::ffi::c_void,
    pub dstPitch: usize,
    pub WidthInBytes: usize,
    pub Height: usize,
}

// --- CUVIDDECODECAPS ---
#[repr(C)]
pub struct CUVIDDECODECAPS {
    pub eCodecType: cudaVideoCodec,
    pub eChromaFormat: u32,
    pub nBitDepthMinus8: u32,
    pub reserved1: [u32; 3],
    pub bIsSupported: u8,
    pub nNumNVDECs: u8,
    pub nOutputFormatMask: u16,
    pub nMaxWidth: u32,
    pub nMaxHeight: u32,
    pub nMaxMBCount: u32,
    pub nMinWidth: u16,
    pub nMinHeight: u16,
    pub bIsHistogramSupported: u8,
    pub nCounterBitDepth: u8,
    pub nMaxHistogramBins: u16,
    pub reserved3: [u32; 10],
}

// --- デコーダ関連構造体 ---
#[repr(C)]
pub struct CUVIDEOFORMAT {
    pub codec: cudaVideoCodec,
    pub frame_rate: _CUVIDEOFORMAT_frame_rate,
    pub progressive_sequence: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub min_num_decode_surfaces: u8,
    pub coded_width: u32,
    pub coded_height: u32,
    pub display_area: _CUVIDEOFORMAT_display_area,
    pub chroma_format: u32,
    pub bitrate: u32,
    pub display_aspect_ratio: _CUVIDEOFORMAT_display_aspect_ratio,
    pub video_signal_description: _CUVIDEOFORMAT_video_signal_description,
    pub seqhdr_data_length: u32,
}
#[repr(C)]
pub struct _CUVIDEOFORMAT_frame_rate { pub numerator: u32, pub denominator: u32 }
#[repr(C)]
pub struct _CUVIDEOFORMAT_display_area { pub left: i16, pub top: i16, pub right: i16, pub bottom: i16 }
#[repr(C)]
pub struct _CUVIDEOFORMAT_display_aspect_ratio { pub x: i32, pub y: i32 }
#[repr(C)]
pub struct _CUVIDEOFORMAT_video_signal_description {
    pub video_format: u8,
    pub video_full_range_flag: u8,
    pub reserved_zero_bits: u8,
    pub color_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
}
#[repr(C)]
pub struct CUVIDPICPARAMS { _private: [u8; 0] }
#[repr(C)]
pub struct CUVIDPARSERDISPINFO {
    pub picture_index: i32,
    pub progressive_frame: i32,
    pub top_field_first: i32,
    pub repeat_first_field: i32,
    pub timestamp: i64,
}
#[repr(C)]
pub struct CUVIDPARSERPARAMS {
    pub CodecType: cudaVideoCodec,
    pub ulMaxNumDecodeSurfaces: u32,
    pub ulClockRate: u32,
    pub ulErrorThreshold: u32,
    pub ulMaxDisplayDelay: u32,
    pub bAnnexb: u32,
    pub uReserved: u32,
    pub uReserved1: [u32; 4],
    pub pUserData: *mut std::ffi::c_void,
    pub pfnSequenceCallback: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut CUVIDEOFORMAT) -> i32>,
    pub pfnDecodePicture: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut CUVIDPICPARAMS) -> i32>,
    pub pfnDisplayPicture: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut CUVIDPARSERDISPINFO) -> i32>,
    pub pfnGetOperatingPoint: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32>,
    pub pfnGetSEIMsg: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32>,
    pub pvReserved2: [*mut std::ffi::c_void; 5],
    pub pExtVideoInfo: *mut std::ffi::c_void,
}
#[repr(C)]
pub struct CUVIDSOURCEDATAPACKET {
    pub flags: u64,
    pub payload_size: u64,
    pub payload: *const u8,
    pub timestamp: i64,
}
#[repr(C)]
pub struct CUVIDDECODECREATEINFO {
    pub ulWidth: u64, pub ulHeight: u64,
    pub ulNumDecodeSurfaces: u64,
    pub CodecType: cudaVideoCodec, pub ChromaFormat: u32,
    pub ulCreationFlags: u64,
    pub bitDepthMinus8: u64,
    pub ulIntraDecodeOnly: u64,
    pub ulMaxWidth: u64, pub ulMaxHeight: u64,
    pub Reserved1: u64,
    pub display_area: _CUVIDDECODECREATEINFO_display_area,
    pub OutputFormat: u32,
    pub DeinterlaceMode: u32,
    pub ulTargetWidth: u64, pub ulTargetHeight: u64,
    pub ulNumOutputSurfaces: u64,
    pub vidLock: CUvideoctxlock,
    pub target_rect: _CUVIDDECODECREATEINFO_target_rect,
    pub enableHistogram: u32,
    pub Reserved2: [u64; 4],
}
#[repr(C)]
pub struct _CUVIDDECODECREATEINFO_display_area { pub left: i16, pub top: i16, pub right: i16, pub bottom: i16 }
#[repr(C)]
pub struct _CUVIDDECODECREATEINFO_target_rect { pub left: i16, pub top: i16, pub right: i16, pub bottom: i16 }
#[repr(C)]
pub struct CUVIDPROCPARAMS {
    pub progressive_frame: i32,
    pub second_field: i32,
    pub top_field_first: i32,
    pub unpaired_field: i32,
    pub reserved_flags: u32,
    pub reserved_zero: u32,
    pub raw_input_dptr: u64,
    pub raw_input_pitch: u32,
    pub raw_input_format: u32,
    pub raw_output_dptr: u64,
    pub raw_output_pitch: u32,
    pub Reserved1: u32,
    pub output_stream: *mut std::ffi::c_void,
    pub Reserved: [u32; 46],
    pub histogram_dptr: *mut u64,
    pub Reserved2: [*mut std::ffi::c_void; 1],
}

// --- デコーダ列挙定数 ---
pub const cudaVideoCodec_enum_cudaVideoCodec_H264: cudaVideoCodec = 4;
pub const cudaVideoCodec_enum_cudaVideoCodec_HEVC: cudaVideoCodec = 8;
pub const cudaVideoCodec_enum_cudaVideoCodec_VP8: cudaVideoCodec = 9;
pub const cudaVideoCodec_enum_cudaVideoCodec_VP9: cudaVideoCodec = 10;
pub const cudaVideoCodec_enum_cudaVideoCodec_AV1: cudaVideoCodec = 11;
pub const cudaVideoCodec_enum_cudaVideoCodec_JPEG: cudaVideoCodec = 5;
pub const cudaVideoChromaFormat_enum_cudaVideoChromaFormat_420: u32 = 1;
pub const cudaVideoSurfaceFormat_enum_cudaVideoSurfaceFormat_NV12: u32 = 0;
pub const cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Weave: u32 = 0;
pub const cudaVideoDeinterlaceMode_enum_cudaVideoDeinterlaceMode_Adaptive: u32 = 2;
pub const cudaVideoCreateFlags_enum_cudaVideoCreate_PreferCUVID: u32 = 2;
pub const CUvideopacketflags_CUVID_PKT_ENDOFSTREAM: u32 = 1;

// --- NVENC 構造体 ---
#[repr(C)]
pub struct NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS {
    pub version: u32,
    pub deviceType: u32,
    pub device: *mut std::ffi::c_void,
    pub reserved: *mut std::ffi::c_void,
    pub apiVersion: u32,
    pub reserved1: u32,
    pub reserved2: [*mut std::ffi::c_void; 64],
}

#[repr(C)]
pub struct NV_ENC_CAPS_PARAM {
    pub version: u32,
    pub capsToQuery: u32,
    pub reserved: [u32; 62],
}

#[repr(C)]
pub struct NV_ENC_PRESET_CONFIG {
    pub version: u32,
    pub presetCfg: NV_ENC_CONFIG,
    pub reserved1: [u32; 255],
    pub reserved2: [*mut std::ffi::c_void; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NV_ENC_CONFIG {
    pub version: u32,
    pub profileGUID: GUID,
    pub gopLength: u32,
    pub frameIntervalP: i32,
    pub monoChromeEncoding: u32,
    pub rcParams: NV_ENC_RC_PARAMS,
    pub encodeCodecConfig: NV_ENC_CODEC_CONFIG,
    pub reserved: [u32; 278],
    pub reserved2: [*mut std::ffi::c_void; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NV_ENC_RC_PARAMS {
    pub version: u32,
    pub rateControlMode: NV_ENC_PARAMS_RC_MODE,
    pub constQP: NV_ENC_QP,
    pub averageBitRate: u32,
    pub maxBitRate: u32,
    pub vbvBufferSize: u32,
    pub vbvInitialDelay: u32,
    pub enableMinQP: u32,
    pub enableMaxQP: u32,
    pub minQP: NV_ENC_QP,
    pub maxQP: NV_ENC_QP,
    pub initialRCQP: NV_ENC_QP,
    pub temporallayerIdxMask: u32,
    pub temporalLayerQP: [u8; 8],
    pub targetQuality: u8,
    pub targetQualityLSB: u8,
    pub lookaheadDepth: u16,
    pub lowDelayKeyFrameScale: u8,
    pub reserved1: [u8; 3],
    pub qpMapMode: u32,
    pub multiPass: u32,
    pub alphaLayerBitrateRatio: u32,
    pub reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NV_ENC_QP { pub qpInterP: u32, pub qpInterB: u32, pub qpIntra: u32 }

#[repr(C)]
#[derive(Clone, Copy)]
pub union NV_ENC_CODEC_CONFIG {
    pub h264Config: NV_ENC_CONFIG_H264,
    pub hevcConfig: NV_ENC_CONFIG_HEVC,
    pub av1Config: NV_ENC_CONFIG_AV1,
    pub reserved: [u32; 320],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NV_ENC_CONFIG_H264 { pub reserved: [u32; 10], pub idrPeriod: u32, pub rest: [u32; 309] }
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NV_ENC_CONFIG_HEVC { pub reserved: [u32; 10], pub idrPeriod: u32, pub rest: [u32; 309] }
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NV_ENC_CONFIG_AV1 { pub reserved: [u32; 10], pub idrPeriod: u32, pub rest: [u32; 309] }

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NV_ENC_INITIALIZE_PARAMS {
    pub version: u32,
    pub encodeGUID: GUID,
    pub presetGUID: GUID,
    pub encodeWidth: u32,
    pub encodeHeight: u32,
    pub darWidth: u32,
    pub darHeight: u32,
    pub frameRateNum: u32,
    pub frameRateDen: u32,
    pub enableEncodeAsync: u32,
    pub enablePTD: u32,
    pub reportSliceOffsets: u32,
    pub enableSubFrameWrite: u32,
    pub presetWidth: u32,
    pub presetHeight: u32,
    pub enableMEOnlyMode: u32,
    pub enableOutputInVidmem: u32,
    pub enableReconFrameOutput: u32,
    pub enableOutputStats: u32,
    pub enableUniDirectionalB: u32,
    pub reserved: u32,
    pub privDataSize: u32,
    pub privData: *mut std::ffi::c_void,
    pub encodeConfig: *mut NV_ENC_CONFIG,
    pub maxEncodeWidth: u32,
    pub maxEncodeHeight: u32,
    pub maxMEHintCountsPerBlock: [u32; 2],
    pub tuningInfo: NV_ENC_TUNING_INFO,
    pub reserved2: [u32; 288],
    pub reserved3: [*mut std::ffi::c_void; 64],
}

#[repr(C)]
pub struct NV_ENC_RECONFIGURE_PARAMS {
    pub version: u32,
    pub reInitEncodeParams: NV_ENC_INITIALIZE_PARAMS,
    pub resetEncoder: u32,
    pub forceIDR: u32,
    pub reserved: [u32; 254],
    pub reserved2: [*mut std::ffi::c_void; 64],
}

type PNVENCODEAPICREATEINSTANCE = Option<unsafe extern "C" fn(*mut NV_ENCODE_API_FUNCTION_LIST) -> u32>;
type PNVENCOPENENCODESESSIONEX = Option<unsafe extern "C" fn(*mut NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS, *mut *mut std::ffi::c_void) -> u32>;
type PNVENCDESTROYENCODER = Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> u32>;
type PNVENCGETENCODECAPS = Option<unsafe extern "C" fn(*mut std::ffi::c_void, GUID, *mut NV_ENC_CAPS_PARAM, *mut i32) -> u32>;
type PNVENCGETENCODEPRESETCONFIGEX = Option<unsafe extern "C" fn(*mut std::ffi::c_void, GUID, GUID, NV_ENC_TUNING_INFO, *mut NV_ENC_PRESET_CONFIG) -> u32>;
type PNVENCINITIALIZEENCODER = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_INITIALIZE_PARAMS) -> u32>;
type PNVENCRECONFIGUREENCODER = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_RECONFIGURE_PARAMS) -> u32>;
type PNVENCREGISTERRESOURCE = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_REGISTER_RESOURCE) -> u32>;
type PNVENCUNREGISTERRESOURCE = Option<unsafe extern "C" fn(*mut std::ffi::c_void, NV_ENC_REGISTERED_PTR) -> u32>;
type PNVENCMAPINPUTRESOURCE = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_MAP_INPUT_RESOURCE) -> u32>;
type PNVENCUNMAPINPUTRESOURCE = Option<unsafe extern "C" fn(*mut std::ffi::c_void, NV_ENC_INPUT_PTR) -> u32>;
type PNVENCCREATEBISTREAMB = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_CREATE_BITSTREAM_BUFFER) -> u32>;
type PNVENCDESTROYBISTREAMB = Option<unsafe extern "C" fn(*mut std::ffi::c_void, NV_ENC_OUTPUT_PTR) -> u32>;
type PNVENCENCODEPICTURE = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_PIC_PARAMS) -> u32>;
type PNVENCLOCKBITSTREAM = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_LOCK_BITSTREAM) -> u32>;
type PNVENCUNLOCKBITSTREAM = Option<unsafe extern "C" fn(*mut std::ffi::c_void, NV_ENC_OUTPUT_PTR) -> u32>;
type PNVENCGETSEQUENCEPARAMS = Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut NV_ENC_SEQUENCE_PARAM_PAYLOAD) -> u32>;

#[repr(C)]
pub struct NV_ENCODE_API_FUNCTION_LIST {
    pub version: u32,
    pub reserved: u32,
    pub nvEncOpenEncodeSessionEx: PNVENCOPENENCODESESSIONEX,
    pub nvEncGetEncodeCaps: PNVENCGETENCODECAPS,
    pub nvEncGetEncodePresetConfigEx: PNVENCGETENCODEPRESETCONFIGEX,
    pub nvEncInitializeEncoder: PNVENCINITIALIZEENCODER,
    pub nvEncReconfigureEncoder: PNVENCRECONFIGUREENCODER,
    pub nvEncRegisterResource: PNVENCREGISTERRESOURCE,
    pub nvEncUnregisterResource: PNVENCUNREGISTERRESOURCE,
    pub nvEncMapInputResource: PNVENCMAPINPUTRESOURCE,
    pub nvEncUnmapInputResource: PNVENCUNMAPINPUTRESOURCE,
    pub nvEncCreateBitstreamBuffer: PNVENCCREATEBISTREAMB,
    pub nvEncDestroyBitstreamBuffer: PNVENCDESTROYBISTREAMB,
    pub nvEncEncodePicture: PNVENCENCODEPICTURE,
    pub nvEncLockBitstream: PNVENCLOCKBITSTREAM,
    pub nvEncUnlockBitstream: PNVENCUNLOCKBITSTREAM,
    pub nvEncGetSequenceParams: PNVENCGETSEQUENCEPARAMS,
    pub nvEncDestroyEncoder: PNVENCDESTROYENCODER,
    pub reserved2: [*mut std::ffi::c_void; 64],
}

#[repr(C)]
pub struct NV_ENC_REGISTER_RESOURCE {
    pub version: u32,
    pub resourceType: u32,
    pub resourceToRegister: *mut std::ffi::c_void,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub subResourceIndex: u32,
    pub bufferFormat: NV_ENC_BUFFER_FORMAT,
    pub bufferUsage: u32,
    pub pInputFencePoint: *mut std::ffi::c_void,
    pub pOutputFencePoint: *mut std::ffi::c_void,
    pub reserved1: [u32; 247],
    pub registeredResource: NV_ENC_REGISTERED_PTR,
    pub reserved2: [*mut std::ffi::c_void; 62],
}
#[repr(C)]
pub struct NV_ENC_MAP_INPUT_RESOURCE {
    pub version: u32,
    pub subResourceIndex: u32,
    pub inputResource: NV_ENC_INPUT_PTR,
    pub registeredResource: NV_ENC_REGISTERED_PTR,
    pub mappedResource: NV_ENC_INPUT_PTR,
    pub mappedBufferFmt: NV_ENC_BUFFER_FORMAT,
    pub reserved1: [u32; 251],
    pub reserved2: [*mut std::ffi::c_void; 63],
}
#[repr(C)]
pub struct NV_ENC_CREATE_BITSTREAM_BUFFER {
    pub version: u32,
    pub size: u32,
    pub memoryHeap: u32,
    pub reserved: u32,
    pub bitstreamBuffer: NV_ENC_OUTPUT_PTR,
    pub bitstreamBufferPtr: *mut std::ffi::c_void,
    pub reserved1: [u32; 254],
    pub reserved2: [*mut std::ffi::c_void; 64],
}
#[repr(C)]
pub struct NV_ENC_PIC_PARAMS {
    pub version: u32,
    pub inputWidth: u32,
    pub inputHeight: u32,
    pub inputPitch: u32,
    pub encodePicFlags: u32,
    pub frameIdx: u32,
    pub inputTimeStamp: u64,
    pub inputDuration: u64,
    pub inputBuffer: NV_ENC_INPUT_PTR,
    pub outputBitstream: NV_ENC_OUTPUT_PTR,
    pub completionEvent: *mut std::ffi::c_void,
    pub bufferFmt: NV_ENC_BUFFER_FORMAT,
    pub pictureStruct: u32,
    pub pictureType: NV_ENC_PIC_TYPE,
    pub codecPicParams: [u32; 256],
    pub reserved: [u32; 234],
    pub reserved2: [*mut std::ffi::c_void; 64],
}
#[repr(C)]
pub struct NV_ENC_LOCK_BITSTREAM {
    pub version: u32,
    pub doNotWait: u32,
    pub ltrFrame: u32,
    pub reservedBitFields: u32,
    pub outputBitstream: *mut std::ffi::c_void,
    pub sliceOffsets: *mut u32,
    pub frameIdx: u32,
    pub hwEncodeStatus: u32,
    pub numSlices: u32,
    pub bitstreamSizeInBytes: u32,
    pub outputTimeStamp: u64,
    pub outputDuration: u64,
    pub bitstreamBufferPtr: *mut std::ffi::c_void,
    pub pictureType: NV_ENC_PIC_TYPE,
    pub pictureStruct: u32,
    pub frameAvgQP: u32,
    pub frameSatd: u32,
    pub ltrFrameIdx: u32,
    pub ltrFrameBitmap: u32,
    pub temporalId: u32,
    pub reserved: [u32; 13],
    pub intraMBCount: u32,
    pub interMBCount: u32,
    pub averageMVX: i32,
    pub averageMVY: i32,
    pub alphaLayerSizeInBytes: u32,
    pub reserved1: [u32; 218],
    pub reserved2: [*mut std::ffi::c_void; 64],
}
#[repr(C)]
pub struct NV_ENC_SEQUENCE_PARAM_PAYLOAD {
    pub version: u32,
    pub inBufferSize: u32,
    pub spsId: u32,
    pub ppsId: u32,
    pub spsppsBuffer: *mut std::ffi::c_void,
    pub outSPSPPSPayloadSize: *mut u32,
    pub reserved: [u32; 250],
    pub reserved2: [*mut std::ffi::c_void; 64],
}

// --- NVENC 定数 ---
pub const NVENCAPI_VERSION: u32 = 0;
pub const NV_ENCODE_API_FUNCTION_LIST_VER: u32 = 0;
pub const NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER: u32 = 0;
pub const NV_ENC_PRESET_CONFIG_VER: u32 = 0;
pub const NV_ENC_CONFIG_VER: u32 = 0;
pub const NV_ENC_INITIALIZE_PARAMS_VER: u32 = 0;
pub const NV_ENC_CREATE_BITSTREAM_BUFFER_VER: u32 = 0;
pub const NV_ENC_PIC_PARAMS_VER: u32 = 0;
pub const NV_ENC_LOCK_BITSTREAM_VER: u32 = 0;
pub const NV_ENC_REGISTER_RESOURCE_VER: u32 = 0;
pub const NV_ENC_MAP_INPUT_RESOURCE_VER: u32 = 0;
pub const NV_ENC_SEQUENCE_PARAM_PAYLOAD_VER: u32 = 0;
pub const NV_ENC_CAPS_PARAM_VER: u32 = 0;
pub const NV_ENC_RECONFIGURE_PARAMS_VER: u32 = 0;
pub const NV_MAX_SEQ_HDR_LEN: u32 = 512;
pub const NVENC_INFINITE_GOPLENGTH: u32 = 0xFFFFFFFF;
pub const NV_ENC_PIC_FLAG_FORCEINTRA: u32 = 0x1;
pub const NV_ENC_PIC_FLAG_FORCEIDR: u32 = 0x2;
pub const NV_ENC_PIC_FLAG_OUTPUT_SPSPPS: u32 = 0x4;
pub const NV_ENC_PIC_FLAG_EOS: u32 = 0x8;

pub const _NV_ENC_DEVICE_TYPE_NV_ENC_DEVICE_TYPE_CUDA: u32 = 1;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_NV12: NV_ENC_BUFFER_FORMAT = 0x00000001;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YV12: NV_ENC_BUFFER_FORMAT = 0x00000010;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_IYUV: NV_ENC_BUFFER_FORMAT = 0x00000100;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YUV444: NV_ENC_BUFFER_FORMAT = 0x00001000;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YUV420_10BIT: NV_ENC_BUFFER_FORMAT = 0x00010000;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_YUV444_10BIT: NV_ENC_BUFFER_FORMAT = 0x00100000;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ARGB: NV_ENC_BUFFER_FORMAT = 0x01000000;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ABGR: NV_ENC_BUFFER_FORMAT = 0x10000000;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ARGB10: NV_ENC_BUFFER_FORMAT = 0x02000000;
pub const _NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_ABGR10: NV_ENC_BUFFER_FORMAT = 0x20000000;
pub const _NV_ENC_BUFFER_USAGE_NV_ENC_INPUT_IMAGE: u32 = 0;
pub const _NV_ENC_INPUT_RESOURCE_TYPE_NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR: u32 = 2;
pub const _NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME: u32 = 1;

pub const _NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_CONSTQP: NV_ENC_PARAMS_RC_MODE = 0;
pub const _NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_VBR: NV_ENC_PARAMS_RC_MODE = 1;
pub const _NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_CBR: NV_ENC_PARAMS_RC_MODE = 2;

pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_P: NV_ENC_PIC_TYPE = 0;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_B: NV_ENC_PIC_TYPE = 1;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_I: NV_ENC_PIC_TYPE = 2;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_IDR: NV_ENC_PIC_TYPE = 3;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_BI: NV_ENC_PIC_TYPE = 4;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_SKIPPED: NV_ENC_PIC_TYPE = 5;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_INTRA_REFRESH: NV_ENC_PIC_TYPE = 6;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_NONREF_P: NV_ENC_PIC_TYPE = 7;
pub const _NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_SWITCH: NV_ENC_PIC_TYPE = 8;

pub const _NVENCSTATUS_NV_ENC_SUCCESS: u32 = 0;
pub const _NVENCSTATUS_NV_ENC_ERR_NO_ENCODE_DEVICE: u32 = 1;
pub const _NVENCSTATUS_NV_ENC_ERR_UNSUPPORTED_DEVICE: u32 = 2;
pub const _NVENCSTATUS_NV_ENC_ERR_INVALID_ENCODERDEVICE: u32 = 3;
pub const _NVENCSTATUS_NV_ENC_ERR_INVALID_DEVICE: u32 = 4;
pub const _NVENCSTATUS_NV_ENC_ERR_DEVICE_NOT_EXIST: u32 = 5;
pub const _NVENCSTATUS_NV_ENC_ERR_INVALID_PTR: u32 = 6;
pub const _NVENCSTATUS_NV_ENC_ERR_INVALID_EVENT: u32 = 7;
pub const _NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM: u32 = 8;
pub const _NVENCSTATUS_NV_ENC_ERR_INVALID_CALL: u32 = 9;
pub const _NVENCSTATUS_NV_ENC_ERR_OUT_OF_MEMORY: u32 = 10;
pub const _NVENCSTATUS_NV_ENC_ERR_ENCODER_NOT_INITIALIZED: u32 = 11;
pub const _NVENCSTATUS_NV_ENC_ERR_UNSUPPORTED_PARAM: u32 = 12;
pub const _NVENCSTATUS_NV_ENC_ERR_LOCK_BUSY: u32 = 13;
pub const _NVENCSTATUS_NV_ENC_ERR_NOT_ENOUGH_BUFFER: u32 = 14;
pub const _NVENCSTATUS_NV_ENC_ERR_INVALID_VERSION: u32 = 15;
pub const _NVENCSTATUS_NV_ENC_ERR_MAP_FAILED: u32 = 16;
pub const _NVENCSTATUS_NV_ENC_ERR_NEED_MORE_INPUT: u32 = 17;
pub const _NVENCSTATUS_NV_ENC_ERR_ENCODER_BUSY: u32 = 18;
pub const _NVENCSTATUS_NV_ENC_ERR_EVENT_NOT_REGISTERD: u32 = 19;
pub const _NVENCSTATUS_NV_ENC_ERR_GENERIC: u32 = 20;
pub const _NVENCSTATUS_NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY: u32 = 21;
pub const _NVENCSTATUS_NV_ENC_ERR_UNIMPLEMENTED: u32 = 22;
pub const _NVENCSTATUS_NV_ENC_ERR_RESOURCE_REGISTER_FAILED: u32 = 23;
pub const _NVENCSTATUS_NV_ENC_ERR_RESOURCE_NOT_REGISTERED: u32 = 24;
pub const _NVENCSTATUS_NV_ENC_ERR_RESOURCE_NOT_MAPPED: u32 = 25;
pub const _NVENCSTATUS_NV_ENC_ERR_NEED_MORE_OUTPUT: u32 = 26;

pub const _NV_ENC_CAPS_NV_ENC_CAPS_SUPPORTED_RATECONTROL_MODES: u32 = 2;
pub const _NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_YUV444_ENCODE: u32 = 3;
pub const _NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_MEONLY_MODE: u32 = 22;
pub const _NV_ENC_CAPS_NV_ENC_CAPS_WIDTH_MAX: u32 = 16;
pub const _NV_ENC_CAPS_NV_ENC_CAPS_HEIGHT_MAX: u32 = 17;
pub const _NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_10BIT_ENCODE: u32 = 26;
pub const _NV_ENC_CAPS_NV_ENC_CAPS_SUPPORT_LOSSLESS_ENCODE: u32 = 18;

// --- GUID 定数 ---
pub const NV_ENC_CODEC_H264_GUID: GUID = GUID { Data1: 0x6bc82762, Data2: 0x4e63, Data3: 0x4ca4, Data4: [0xaa, 0x85, 0x1e, 0x50, 0xf3, 0x21, 0xf6, 0xbf] };
pub const NV_ENC_CODEC_HEVC_GUID: GUID = GUID { Data1: 0x790cdc88, Data2: 0x4522, Data3: 0x4d7b, Data4: [0x94, 0x25, 0xbd, 0xa9, 0x97, 0x5f, 0x76, 0x03] };
pub const NV_ENC_CODEC_AV1_GUID: GUID = GUID { Data1: 0x0a352289, Data2: 0x0aa7, Data3: 0x4759, Data4: [0x86, 0x2d, 0x5d, 0x15, 0xcd, 0x16, 0xd2, 0x54] };
pub const NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID: GUID = GUID { Data1: 0xbfd6f8e7, Data2: 0x233c, Data3: 0x4341, Data4: [0x8b, 0x3e, 0x48, 0x18, 0x52, 0x38, 0x03, 0xf4] };
pub const NV_ENC_H264_PROFILE_BASELINE_GUID: GUID = GUID { Data1: 0x0727bcaa, Data2: 0x78c4, Data3: 0x4c83, Data4: [0x8c, 0x2f, 0xef, 0x3d, 0xff, 0x26, 0x7c, 0x6a] };
pub const NV_ENC_H264_PROFILE_MAIN_GUID: GUID = GUID { Data1: 0x60b5c1d4, Data2: 0x67fe, Data3: 0x4790, Data4: [0x94, 0xd5, 0xc4, 0x72, 0x6d, 0x7b, 0x6e, 0x6d] };
pub const NV_ENC_H264_PROFILE_HIGH_GUID: GUID = GUID { Data1: 0xe7cbc309, Data2: 0x4f7a, Data3: 0x4b89, Data4: [0xaf, 0x2a, 0xd5, 0x37, 0xc9, 0x2b, 0xe3, 0x10] };
pub const NV_ENC_H264_PROFILE_HIGH_10_GUID: GUID = GUID { Data1: 0x8f0c337e, Data2: 0x186c, Data3: 0x48e9, Data4: [0xa6, 0x9d, 0x7a, 0x83, 0x34, 0x08, 0x97, 0x58] };
pub const NV_ENC_H264_PROFILE_HIGH_422_GUID: GUID = GUID { Data1: 0xff3242e9, Data2: 0x613c, Data3: 0x4295, Data4: [0xa1, 0xe8, 0x2a, 0x7f, 0xe9, 0x4d, 0x81, 0x33] };
pub const NV_ENC_H264_PROFILE_HIGH_444_GUID: GUID = GUID { Data1: 0x7ac663cb, Data2: 0xa598, Data3: 0x4960, Data4: [0xb8, 0x44, 0x33, 0x9b, 0x26, 0x1a, 0x7d, 0x52] };
pub const NV_ENC_H264_PROFILE_STEREO_GUID: GUID = GUID { Data1: 0x40847bf5, Data2: 0x33f7, Data3: 0x4601, Data4: [0x90, 0x84, 0xe8, 0xfe, 0x3c, 0x1d, 0xb8, 0xb7] };
pub const NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID: GUID = GUID { Data1: 0xb405afac, Data2: 0xf32b, Data3: 0x417b, Data4: [0x89, 0xc4, 0x9a, 0xbe, 0xed, 0x3e, 0x59, 0x78] };
pub const NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID: GUID = GUID { Data1: 0xaec1bd87, Data2: 0xe85b, Data3: 0x48f2, Data4: [0x84, 0xc3, 0x98, 0xbc, 0xa6, 0x28, 0x50, 0x72] };
pub const NV_ENC_HEVC_PROFILE_MAIN_GUID: GUID = GUID { Data1: 0xb514c39a, Data2: 0xb55b, Data3: 0x40fa, Data4: [0x87, 0x8f, 0xf1, 0x25, 0x3b, 0x4d, 0xfd, 0xec] };
pub const NV_ENC_HEVC_PROFILE_MAIN10_GUID: GUID = GUID { Data1: 0xfa4d2b6c, Data2: 0x3a5b, Data3: 0x411a, Data4: [0x80, 0x18, 0x0a, 0x3f, 0x5e, 0x3c, 0x9b, 0xe5] };
pub const NV_ENC_HEVC_PROFILE_FREXT_GUID: GUID = GUID { Data1: 0x51ec32b5, Data2: 0x1b4c, Data3: 0x453c, Data4: [0x9c, 0xbd, 0xb6, 0x16, 0xbd, 0x62, 0x13, 0x41] };
pub const NV_ENC_AV1_PROFILE_MAIN_GUID: GUID = GUID { Data1: 0x5f2a39f5, Data2: 0xf14e, Data3: 0x4f95, Data4: [0x9a, 0x9e, 0xb7, 0x6d, 0x56, 0x8f, 0xcf, 0x97] };
pub const NV_ENC_PRESET_P1_GUID: GUID = GUID { Data1: 0xfc0a8d3e, Data2: 0x45f8, Data3: 0x4cf8, Data4: [0x80, 0xc7, 0x29, 0x88, 0x71, 0x59, 0x0e, 0xbf] };
pub const NV_ENC_PRESET_P2_GUID: GUID = GUID { Data1: 0xf581cfb8, Data2: 0x88d6, Data3: 0x4381, Data4: [0x93, 0xf0, 0xdf, 0x13, 0xf9, 0xc2, 0x7d, 0xab] };
pub const NV_ENC_PRESET_P3_GUID: GUID = GUID { Data1: 0x36850110, Data2: 0x3a07, Data3: 0x441f, Data4: [0x94, 0xd5, 0x36, 0x70, 0x63, 0x1f, 0x91, 0xf6] };
pub const NV_ENC_PRESET_P4_GUID: GUID = GUID { Data1: 0x90a7b826, Data2: 0xdf06, Data3: 0x4862, Data4: [0xb9, 0xd2, 0xcd, 0x6d, 0x73, 0xa0, 0x86, 0x81] };
pub const NV_ENC_PRESET_P5_GUID: GUID = GUID { Data1: 0x21c6e6b4, Data2: 0x297a, Data3: 0x4cba, Data4: [0x99, 0x8f, 0xb6, 0xcb, 0xde, 0x72, 0xad, 0xe3] };
pub const NV_ENC_PRESET_P6_GUID: GUID = GUID { Data1: 0x8e75c279, Data2: 0x6299, Data3: 0x4ab6, Data4: [0x83, 0x02, 0x0b, 0x21, 0x5a, 0x33, 0x5c, 0xf5] };
pub const NV_ENC_PRESET_P7_GUID: GUID = GUID { Data1: 0x84848c12, Data2: 0x6f71, Data3: 0x4c13, Data4: [0x93, 0x1b, 0x53, 0xe2, 0x83, 0xf5, 0x79, 0x74] };
pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY: NV_ENC_TUNING_INFO = 1;
pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOW_LATENCY: NV_ENC_TUNING_INFO = 2;
pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY: NV_ENC_TUNING_INFO = 3;
pub const NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOSSLESS: NV_ENC_TUNING_INFO = 4;
"#
    .to_string()
}
