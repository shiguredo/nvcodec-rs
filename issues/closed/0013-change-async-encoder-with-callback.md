# 0013-change-async-encoder-with-callback

Created: 2026-05-06
Completed: 2026-05-09
Model: deepseek-v4-pro

## 背景

nvcodec-rs のエンコード処理は従来、`Encoder::encode()` が同期的に `NvEncEncodePicture` → `NvEncLockBitstream` までをブロッキングで実行する設計になっていた。
呼び出し元スレッドがエンコード完了までブロックされるため、低遅延が求められるユースケース（リアルタイム配信等）ではエンコード完了を別スレッドで待機し、
完了次第即座にコールバックを呼び出す非同期モデルが必要だった。

NVENC API には以下の非同期関連機構が存在する:

| 機構 | Linux 対応 |
|------|-----------|
| `enableEncodeAsync` + `NvEncRegisterAsyncEvent` + `completionEvent` | **非対応** (nvEncodeAPI.h L3393 に明記) |
| `NvEncSetIOCudaStreams` | 対応 |
| `NV_ENC_LOCK_BITSTREAM::doNotWait` | 対応 |

一方、同期モードでも API ドキュメントに "The client working in synchronous mode can work in a single threaded or multi threaded mode" とある通り、
`NvEncEncodePicture` と `NvEncLockBitstream` を別スレッドから呼び出すことは仕様上許容されている。

## 設計判断

SDK サンプル (`NvEncoder`) のバッファ管理パターン（固定数バッファプール + 剰余循環 + delay window 制御）を踏襲しつつ、
encode 処理全体を専用ワーカースレッド上で実行する方式を採用した。

- `n_encoder_buffer = frameIntervalP + 3`
- `n_output_delay = n_encoder_buffer - 1`
- ワーカースレッドが CUDA context を排他的に所有し、`cuCtxPushCurrent`/`cuCtxPopCurrent` で管理
- `encode()` は `std::sync::mpsc::SyncSender` 経由で job を送信し即座に戻る
- `reconfigure()` は sync channel 経由で job を送信し、ワーカースレッド上で再構成を実行して完了を待つ
- `get_sequence_params()` は sync channel 経由で SPS/PPS (または Sequence Header OBU) を取得する
- ワーカースレッド内で `NvEncEncodePicture` → delay window 制御 → `NvEncLockBitstream`(block) → callback 即呼出し
- `flush()` は全 pending frame の drain を待機。EOS なしで複数回呼出し可能
- `Drop` 時に EOS 送信 → 残り全 drain → スレッド join

## 変更内容

- `Encoder<T>` を新規実装。旧 `Encoder` は廃止
- `EncodedFrame<T>` に `user_data: T` を追加。エンコード完了時に任意のユーザーデータを callback 経由で受け取れる
- `reconfigure()` は旧 `Encoder` に存在していた関数。`SyncSender`/`Receiver` 経由でワーカースレッドに依頼しブロッキング待機する方式に変更
- `get_sequence_params()` は旧 `Encoder` に存在していた関数。同様に sync channel 経由で取得する方式に変更
- `query_caps()` は旧 `Encoder` に存在していたメソッド。standalone 関数に変更し、内部実装は `EncoderState` に移動
- `codec_info.rs` 内の呼出しを `Encoder::query_caps()` → `crate::encode::query_caps()` に修正
- テストを全書き換え（12 tests: init × 3, get_sequence_params × 3, encode black frame × 3, multiple frames, flush without encode, reconfigure）
- 依存追加なし。`std::sync::mpsc` のみ使用

## 解決方法

旧 `Encoder` を廃止し、非同期コールバック方式の `Encoder<T>` を新規実装した。

内部では SDK サンプル (`NvEncoder`) のバッファプール + delay window 制御を踏襲し、エンコード処理全体を専用のワーカースレッド上で実行する。
`encode()` は `std::sync::mpsc::SyncSender` 経由でジョブを送信し即座に戻る。
ワーカースレッドは `NvEncEncodePicture` → delay window 制御 → `NvEncLockBitstream`(block) → callback 即呼出し の順で処理する。
`reconfigure()`、`get_sequence_params()`、`query_caps()` はチャンネルベースまたは standalone 関数として再実装した。
`EncodedFrame<T>` に `user_data: T` を追加し、エンコード完了時に任意のユーザーデータを callback 経由で受け取れるようにした。
