# 0011 extern "C" コールバックに catch_unwind を入れる

## 優先度

P1

## 概要

`src/decode.rs` の `extern "C"` コールバック群 (`handle_video_sequence`, `handle_picture_decode`,
`handle_picture_display`) に `catch_unwind` が入っていない。

コールバック内では `Mutex::lock()` (poisoned で panic)、`Vec` 確保 (OOM で panic)、
整数演算 (debug ビルドでオーバーフロー panic) を通っており、
panic が発生すると FFI 境界を越えてプロセス abort に直結する。

## 対応方針

各 `extern "C"` コールバックの本体を `catch_unwind` で包み、
panic 時はエラーコードを返す。

## 完了内容

`handle_video_sequence` / `handle_picture_decode` / `handle_picture_display` の 3 つの
`extern "C"` コールバックを `catch_unwind(AssertUnwindSafe(...))` で包んだ。
panic 時は `unwrap_or(0)` でエラーコード 0 を返す。
