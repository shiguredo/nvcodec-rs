# cuda.h スタブヘッダに CUdeviceptr の定義が不足している

## 概要

`third_party/cuda/include/cuda.h` スタブヘッダに `CUdeviceptr` の typedef が含まれていないため、
CUDA Toolkit がインストールされていない環境で bindgen によるバインディング生成後のコンパイルが失敗する。

## 背景

- このスタブヘッダは CUDA Toolkit がない環境 (macOS など) で bindgen を動作させるために用意されている
- 元々 build.rs に別のビルドエラー (`toml` クレート未解決) があったため、bindgen まで到達せず問題が表面化していなかった
- build.rs の修正後、`sys::CUdeviceptr` が未定義というコンパイルエラーが発生する

## エラーメッセージ

```
error[E0425]: cannot find type `CUdeviceptr` in module `sys`
```

`src/lib.rs` と `src/encode.rs` の計 15 箇所で発生。

## 修正方針

`third_party/cuda/include/cuda.h` に以下の typedef を追加する:

```c
typedef unsigned long long CUdeviceptr;
```

NVIDIA CUDA Driver API の 64bit 環境向け定義に準拠。
現代の対象環境 (x86_64, aarch64) はすべて 64bit なのでこの定義で問題ない。

## 参考

- NVIDIA Video Codec SDK 13.0.19 の `cuviddec.h` で `CUdeviceptr` が使用されている
- NVIDIA 公式では 32bit/64bit で条件分岐しているが、本プロジェクトの対象環境は 64bit のみ
