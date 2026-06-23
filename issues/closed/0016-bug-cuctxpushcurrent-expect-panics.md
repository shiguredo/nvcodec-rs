# 0016-bug-cuctxpushcurrent-expect-panics

Completed: 2026-05-16
Branch: feature/change-unify-cuda-context
Created: 2026-05-10
Model: DeepSeek v4-pro

## 背景

`run_worker` と `drain_one_with_ctx` 内で `cuCtxPushCurrent` の戻り値に `.expect()` を使っている。
GPU リセットやドライバ障害でコンテキスト操作が失敗した場合、ワーカースレッドがパニックしプロセス全体がアボートする。

根本原因は `EncoderState` のメソッド間で CUDA コンテキスト管理の責務が分散していることにある。
`reconfigure` と `get_sequence_params` は内部で `with_context` を呼んで自己完結しているが、
`encode_frame`、`lock_and_copy_bitstream`、`send_eos` はコンテキストを**呼び出し元任せ**にしており、
`run_worker` と `drain_one_with_ctx` が手動で `cuCtxPushCurrent` / `cuCtxPopCurrent` を行っている。
この設計不整合が `.expect()` パニックの根本原因である。

## 問題箇所

`.expect()` を使っている 3 箇所と、エラーを握り潰している 1 箇所:

```rust
// encode.rs:1397-1398 (run_worker 内の encode パス)
lib.cu_ctx_push_current(ctx)
    .expect("cuCtxPushCurrent in encode");

// encode.rs:1444-1445 (run_worker 内の terminate パス)
lib.cu_ctx_push_current(ctx)
    .expect("cuCtxPushCurrent in terminate");

// encode.rs:1473-1474 (drain_one_with_ctx 内の lock パス)
lib.cu_ctx_push_current(ctx)
    .expect("cuCtxPushCurrent in drain_one");

// encode.rs:1419 (run_worker 内の encode_frame 失敗時のエラーパス)
let _ = lib.cu_ctx_push_current(ctx);
// cuCtxPushCurrent の失敗を無視して unmap_resource を呼んでいる。
// コンテキスト未設定状態での unmap_resource は未定義動作の可能性がある。
```

また `drain_one_with_ctx` の unmap パス (`encode.rs:1504`) でも `let _ = lib.cu_ctx_push_current(ctx);` とエラーを握り潰しているが、こちらの issue は issue 0018 の担当範囲とする。

## 問題

ワーカースレッドがパニックすると:

1. **`EncoderState::drop` は実行されるが、内部のクリーンアップに失敗する**:
   Rust の unwind により `EncoderState::Drop` は実行される。しかし `EncoderState::Drop` 内の
   `self.lib.with_context(self.ctx, ...)` は再度 `cuCtxPushCurrent` を呼ぶ。
   元のパニック原因が `cuCtxPushCurrent` の失敗であるため、Drop 内の `cuCtxPushCurrent` も同様に失敗し、
   `nvEncDestroyEncoder` も `cu_ctx_destroy` も実行されず、CUDA リソースがリークする。

2. **利用者はパニックの原因を知ることができない**:
   `Encoder::Drop` (`encode.rs:1274-1281`) は `let _ = worker.join();` と結果を握り潰している。
   `JoinHandle::join()` はスレッドがパニックした場合 `Err(Box<dyn Any>)` を返すが、
   `let _ =` でこのエラーが捨てられている。利用者はワーカースレッドが正常終了したのか
   パニックしたのかを区別できない。

3. **ワーカースレッド panic 後に `Encoder` の操作がすべてエラーになるが、原因は区別できない**:
   ワーカースレッドが panic で終了すると `job_rx` が drop されて channel が切断される。
   `encode()`、`flush()`、`reconfigure()`、`get_sequence_params()` はいずれも `.send()` 失敗として
   `"send failed"` エラーを返すが、これが panic による異常終了なのか正常な `Terminate` 処理なのかを
   呼び出し元が区別する手段がない。

4. **プロセス全体がアボートするため、他のエンコーダー/デコーダーも巻き込まれる**

## エッジケース

| シナリオ | 問題 |
|---|---|
| encode パス (L1397) で push 失敗 | `encode_frame` が未実行のため unmap 不要。しかし後続の drain (L1390, L1412) で再度 push に失敗し無限エラーループに陥る |
| terminate パス (L1444) で push 失敗 | `send_eos` が未実行。残りフレームの drain (L1451-1453) に進んだ場合、L1473 で再度 push 失敗し無限ループ |
| drain lock パス (L1473) で push 失敗 | lock 前なので unmap 不要だが、`i_got` を進めるべきかどうか（進めないと drain ループが無限になる） |
| encode_frame 失敗パス (L1419) で push 失敗 | `let _ =` で握り潰し、コンテキスト未設定のまま `unmap_resource` を実行し未定義動作の可能性 |
| drain unmap パス (L1504) で push 失敗 | `let _ =` で握り潰し、コンテキスト未設定のまま `unmap_resource` を実行し未定義動作の可能性 |

## 修正方針

**本 issue の修正は issue 0019 の `with_context` 統一によって実現する。**

issue 0019 で `encode_frame`、`lock_and_copy_bitstream`、`send_eos` が `with_context` パターンに
統一されれば、`run_worker` と `drain_one_with_ctx` から手動の `cuCtxPushCurrent` / `cuCtxPopCurrent` が
消滅する。これにより本 issue が対象とする `.expect()` 箇所もすべて消える。

`with_context` (`lib.rs:635-658`) は issue 0012 で `ReleaseGuard` による panic 安全性が確保済みであり、
`Result<(), Error>` を返すため `.expect()` を必要としない。

## 後方互換

公開 API への変更は発生しない。ワーカースレッド内の実装変更のみであり、`[FIX]` に分類される。

## テスト戦略

- **単体テスト**（`src/encode.rs` の `#[cfg(test)] mod tests`）:
  - `cuCtxPushCurrent` が失敗する状況をモックする方法が現状存在しないため、
    直接的なエラーパスのテストは issue 0019 の実装完了後に `with_context` のエラーパスとして検証する
  - ワーカースレッドの終了後の `encode()` / `flush()` 呼び出しがエラーになることのテストは
    issue 0022 の担当範囲とする
- PBT / Fuzzing は不要（この修正は既存の `with_context` パターンの適用範囲拡大であり、
  新たなロジックを追加しないため）

## 解決方法

issue 0019 (`with_context` 統一) の完了により、`run_worker` と `drain_one_with_ctx` 内の手動 `cuCtxPushCurrent` / `cuCtxPopCurrent` 呼び出しがすべて除去された。これにより本 issue が対象としていた `.expect()` パニックの温床もすべて消滅した。

`encode_frame`, `lock_and_copy_bitstream`, `send_eos`, `unmap_resource` の各メソッドが `with_context` で自己完結するようになり、エラーは `Result` 経由で適切に伝播する。
