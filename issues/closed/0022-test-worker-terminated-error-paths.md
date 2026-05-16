# 0022-test-worker-terminated-error-paths

Completed: 2026-05-16
Branch: feature/test-worker-terminated-error-paths
Created: 2026-05-10
Model: deepseek-v4-pro

## 背景

`Encoder<T>` と `Decoder<T>` の各メソッドは、ワーカースレッド終了後に呼び出された場合に `job_tx.send()` が失敗し、`Error::new_custom()` でエラーを返すが、これらのエラーパスに対するテストが一切存在しない。

## 問題箇所

### Encoder 側（`src/encode.rs`）

| メソッド | 行 | エラーメッセージ |
|---------|----|--------------|
| `encode()` | 1224-1232 | `"encoder worker thread has terminated"` |
| `flush()` | 1238-1246 | `"send failed"` |
| `reconfigure()` | 1252-1259 | `"send failed"` |
| `get_sequence_params()` | 1264-1271 | `"send failed"` |

### Decoder 側（`src/decode.rs`）

| メソッド | 行 | エラーメッセージ |
|---------|----|--------------|
| `decode()` | 344-351 | `"decoder worker thread has terminated"` |
| `flush()` | 357-365 | `"send failed"` |

`flush()`、`reconfigure()`、`get_sequence_params()` には `recv` エラーパス（`rx.recv().map_err(...)`）もあるが、ワーカースレッド終了時には `send` が先に失敗するため、本 issue では send エラーパスのみを対象とする。

## テスト実現方法

Rust の所有権モデル上、`drop()` に渡した値に対してメソッドを呼び出すことはできない。`Encoder<T>` と `Decoder<T>` は `Drop` 実装でワーカーを終了させるため、`std::mem::ManuallyDrop` を使用して手動で `Drop` を発火させ、ワーカー終了後も Encoder/Decoder にアクセスできる状態を作る。

`ManuallyDrop<T>` はラップした値の `Drop` を抑止するため、途中で `ManuallyDrop::drop()` を呼んでも値のメモリは解放されず、引き続きアクセス可能である。

以下は Encoder 側のテストコード例。Decoder 側も同様のパターンで実装する:

```rust
use std::mem::ManuallyDrop;
use std::sync::mpsc;

// 既存の test_encoder_config() ヘルパーで H.264 エンコーダ設定を生成
let config = test_encoder_config(CodecConfig::H264(H264EncoderConfig {
    profile: None,
    idr_period: None,
}));

// 既存テストと同様の channel + callback パターン
let (tx, _rx) = mpsc::sync_channel::<Result<EncodedFrame<()>, Error>>(4);
let callback = move |frame| {
    let _ = tx.send(frame);
};

let mut encoder = ManuallyDrop::new(Encoder::new(config, callback).unwrap());

// 手動で Drop を発火: ワーカーが終了しチャネルが閉じる
unsafe { ManuallyDrop::drop(&mut encoder); }

// ワーカー終了後も ManuallyDrop 越しに encoder へアクセス可能
// encode/decode に渡すデータは send 失敗時に内容が処理されないため任意の値でよい
let result = encoder.encode(
    &[],
    &EncodeOptions {
        force_intra: false,
        force_idr: false,
        output_spspps: false,
    },
    (),
);
assert!(result.is_err());

// テスト終了時: Encoder::Drop は発火済みのため forget でメモリをリークさせる。
// あるいは 2 回目の ManuallyDrop::drop を呼んでもよい
// （Encoder::Drop は worker.take() 済みで冪等だが、ドロップの二重実行に注意）。
std::mem::forget(encoder);
```

`reconfigure()` と `get_sequence_params()` は `&mut self` を取るが、`ManuallyDrop<T>` は `DerefMut` を実装しているため auto-deref で呼び出せる。そのため `let mut encoder = ManuallyDrop::new(...)` のように `mut` を付ける。

## テスト項目

テスト種別は「意図的エラーパス」のため単体テストに分類される。テストは既存の `#[cfg(test)] mod tests` ブロックに追加する。

各テストでは `Error::to_string()` でエラーメッセージを検証する。`Error` の `function` フィールドは非公開であり、エラーの種類の判別は文字列比較で行う必要がある。期待される `to_string()` の値は下表のとおり（`Error::Display` 実装は `"{function}() failed: {message}"` の形式）。

| メソッド | 期待される `to_string()` |
|---------|----------------------|
| `Encoder::encode()` | `"encode() failed: encoder worker thread has terminated"` |
| `Encoder::flush()` (send) | `"flush() failed: send failed"` |
| `Encoder::reconfigure()` (send) | `"reconfigure() failed: send failed"` |
| `Encoder::get_sequence_params()` (send) | `"get_sequence_params() failed: send failed"` |
| `Decoder::decode()` | `"decode() failed: decoder worker thread has terminated"` |
| `Decoder::flush()` (send) | `"flush() failed: send failed"` |

### Encoder 側（`src/encode.rs` の `#[cfg(test)] mod tests` に追加、L1512-）

既存の `test_encoder_config()` ヘルパー関数（L1518-1536）を使用し、コーデックは H.264 を指定する。

1. **`test_encode_after_worker_terminated`**: `Encoder<()>` を生成し `ManuallyDrop` でラップする。`ManuallyDrop::drop()` でワーカーを終了させた後、`encode()` を呼び出し、`to_string()` が表の期待値と一致することを検証する。
2. **`test_flush_after_encoder_worker_terminated`**: 同様にワーカー終了後に `flush()` を呼び出し、期待値と一致することを検証する。
3. **`test_reconfigure_after_encoder_worker_terminated`**: 同様にワーカー終了後に `ReconfigureParams::default()` を引数にして `reconfigure()` を呼び出し、期待値と一致することを検証する。
4. **`test_get_sequence_params_after_encoder_worker_terminated`**: 同様にワーカー終了後に `get_sequence_params()` を呼び出し、期待値と一致することを検証する。

### Decoder 側（`src/decode.rs` の `#[cfg(test)] mod tests` に追加、L803-）

既存の `test_decoder_config()` ヘルパー関数（L809-817）を使用し、コーデックは H.264 を指定する。

5. **`test_decode_after_worker_terminated`**: `Decoder<()>` を生成し `ManuallyDrop` でラップする。`ManuallyDrop::drop()` でワーカーを終了させた後、`decode()` を呼び出し、`to_string()` が表の期待値と一致することを検証する。
6. **`test_flush_after_decoder_worker_terminated`**: 同様にワーカー終了後に `flush()` を呼び出し、期待値と一致することを検証する。

## API 互換性

テスト追加のみのため、公開 API に変更はない。後方互換性への影響はない。

## CHANGES.md

テスト追加のみのため、記載は不要。

## 解決方法

`src/encode.rs` と `src/decode.rs` の `#[cfg(test)] mod tests` に、ワーカースレッド終了後のエラーパスを検証する単体テストを追加した。

Encoder 側（4 件）:
- `test_encode_after_worker_terminated`:  encode() が `"encoder worker thread has terminated"` エラーを返すことを確認
- `test_flush_after_encoder_worker_terminated`: flush() が `"send failed"` エラーを返すことを確認
- `test_reconfigure_after_encoder_worker_terminated`: reconfigure() が `"send failed"` エラーを返すことを確認
- `test_get_sequence_params_after_encoder_worker_terminated`: get_sequence_params() が `"send failed"` エラーを返すことを確認

Decoder 側（2 件）:
- `test_decode_after_worker_terminated`: decode() が `"decoder worker thread has terminated"` エラーを返すことを確認
- `test_flush_after_decoder_worker_terminated`: flush() が `"send failed"` エラーを返すことを確認

全テストで `ManuallyDrop` を使用してワーカースレッドを手動終了させ、終了後にメソッド呼び出しを行うパターンを採用した。
