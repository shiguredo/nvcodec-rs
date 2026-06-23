# 0014-change-async-decode-callback

Created: 2026-05-09
Completed: 2026-05-09
Model: deepseek-v4-pro

## 背景

現在のデコーダは `decode()` → `next_frame()` のポーリング方式で実装されている。利用側は `next_frame()` をループで呼び出す必要があり、デコード完了をビジーウェイトまたはポーリング間隔による遅延を伴って待つことになる。

エンコーダはすでに非同期コールバック方式（専用ワーカースレッド + `FnMut` コールバック）に移行済みであり、デコーダも同様のパターンに統一することで API の一貫性と低レイテンシを実現する。

## 目的

- デコーダの API をエンコーダと同じ非同期コールバック方式に変更する
- デコード完了からコールバック呼び出しまでのレイテンシを最小化する
- `Decoder::query_caps` をスタンドアロン関数に移動し、`Encoder::query_caps` と一貫性を持たせる

## 設計判断

`nvcuvid.h` のドキュメントによると、FFI コールバックは `cuvidParseVideoData()` の呼び出し内で同期的に実行される。このため `cuvidParseVideoData()` が戻った時点で全ての FFI コールバックは完了している。

ワーカースレッド上で `cuvidParseVideoData()` → FFI コールバック → channel 送信 → `next_frame()` で drain という流れにしても、全ての処理が同一スレッド上で同期的に完結するためレイテンシの増加はない。

### 実装方針

元の `Decoder` の実装を可能な限り維持するため、以下の構造を採用した:

1. 元の `Decoder` 構造体を `DecoderState` に改名し、旧 `DecoderState` のフィールド（`decoder`, `width`, `height` など）を統合する
2. `DecoderState` はワーカースレッドが単独所有するため `Mutex` は不要（`Box<DecoderState>` を直接扱う）
3. FFI コールバックは `pUserData` として `*mut DecoderState` を受け取り、`&mut *` で直接アクセスする
4. `impl DecoderState` は元の `impl Decoder` の構造をほぼそのまま継承する
5. 新設の `Decoder<T>` は薄い非同期ラッパー: `job_tx: SyncSender<Job<T>>` + `worker: JoinHandle<()>`
6. `run_worker` は `Box<DecoderState>` を受け取り、`state.decode()`, `state.send_eos()`, `state.next_frame()` を直接呼び出す

## API 変更

- `Decoder::new(config)` → `Decoder::new(config, callback)` コールバッククロージャ必須
- `decode(data)` → `decode(data, user_data)` ユーザーデータ必須
- `next_frame()` → 廃止（コールバック駆動に変更）
- `finish()` → `flush()` に変更（エンコーダと命名統一）
- `Decoder::query_caps()` → `query_caps()` スタンドアロン関数に移動
- `DecodedFrame` → `DecodedFrame<T>` に変更、`user_data: T` フィールド追加

## 変更内容

- `src/decode.rs`
  - `Decoder` → `Decoder<T>` + `DecoderState` に分割
  - `DecoderState` は元の `Decoder` 実装をほぼそのまま継承
  - `new_with_codec` は `Box<DecoderState>` を返し、`frame_rx` をフィールドに含める
  - FFI コールバックのユーザーデータアクセスを `&mut *` に簡略化（Mutex 廃止）
  - ワーカースレッド `run_worker`、`drain_frames` を追加
  - `RawFrame` 内部型を追加
  - `DecodedFrame<T>` に `user_data: T` を追加
  - decode 時に `CUVID_PKT_ENDOFPICTURE` フラグを指定するように変更
- `src/lib.rs` - `decode`/`encode` モジュールを `pub` に変更
- `src/codec_info.rs` - `Decoder::query_caps` → `crate::decode::query_caps`
- `README.md` - エンコード/デコード使用例、query_caps 例を全修正
- `CHANGES.md` - develop セクションに追記

## 解決方法

元の `Decoder` 構造体を `DecoderState` に改名し、旧 `DecoderState` のフィールドを統合した。`DecoderState` はワーカースレッドが単独所有するため Mutex を廃止し、FFI コールバックは `pUserData` として `*mut DecoderState` を直接受け取る方式に変更した。

`impl DecoderState` は元の `impl Decoder` のメソッド構造をほぼそのまま継承し、`decode()`、`send_eos()`（元 `finish()`）、`next_frame()` をワーカースレッドから直接呼び出す設計にした。これにより差分を最小限に抑えた。

新設の `Decoder<T>` はエンコーダと同様の薄い非同期ラッパーとし、`job_tx: SyncSender<Job<T>>` と `worker: JoinHandle<()>` のみを持つ。`decode(data, user_data)` はジョブを送信して即座に戻り、デコード完了時にワーカースレッド上でコールバックが呼び出される。
