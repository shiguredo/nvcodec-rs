# 0030-refactor-decode-worker-loop

- Created: 2026-08-14
- Completed: 2026-08-14
- Branch: feature/refactor-decode-worker-loop
- Polished: {YYYY-MM-DD} (例: 2024-07-15)
- Reporter: @sile

## 目的

デコードワーカーのループ (`src/decode.rs` の `run_worker`) が長くなり、終端遷移の処理が複数箇所に重複している。ワーカーの状態を構造体にまとめ、ジョブ種別ごとの処理をメソッドに分離して可読性を高める。

## 現状

`run_worker` はローカル変数として `pending_user_data` / `terminated` を持ち、`state` (`Box<DecoderState>`) / `handler` (`H: DecodeHandler`) と合わせて 4 つの値が関数やヘルパー間を引数で渡り歩く。

また、次の終端遷移パターンが 4 箇所に重複している:

```rust
terminated = true;
discard_queued_frames(&state);
pending_user_data.clear();
```

- `Job::Decode` 内で `DecoderState::decode` が `Err` を返した場合
- `Job::Decode` 内で `drain_frames` が `false` (missing user data) を返した場合
- `Job::Flush` 内で `send_eos` が `Err` を返した場合
- `Job::Flush` 内で `drain_frames` が `false` (missing user data) を返した場合

さらに、`drain_frames` は `(&state, &mut handler, &mut pending_user_data)` の 3 引数で状態を渡すため、呼び出し側と実装側の両方で引き回しの負荷が大きい。

## 設計方針

`DecodeWorker<H>` 構造体を導入し、ワーカーの状態をフィールドにまとめる。

- `state: Box<DecoderState>`
- `handler: H`
- `pending_user_data: VecDeque<H::UserData>`
- `terminated: bool`

終端遷移を `enter_terminated(&mut self)` メソッドに集約する。`on_decoded(Err(...))` を伴う場合も、呼び出し側でこのメソッドの後に呼ぶ。

ジョブ種別ごとの処理をメソッドに分離する。

- `handle_decode(&mut self, data: &[u8], user_data: H::UserData)`
- `handle_flush(&mut self, done: SyncSender<()>)`
- `handle_terminate(&mut self)`

`run_worker` は `DecodeWorker` を構築し、`job_rx` から受け取った `Job` を各メソッドへ dispatch するだけにする。

現行の `run_worker` は `Ok(Job::Terminate) | Err(_)` を同一アームで扱っている。構造体化で `while let Ok(job) = job_rx.recv()` に変更する場合、チャネル破棄 (`Err(_)`) 時の後始末 (`handle_terminate` 相当) も行えるように注意する。

## 完了条件

- 終端遷移パターンの重複が `enter_terminated` メソッド 1 箇所に集約されている
- `state` / `handler` / `pending_user_data` / `terminated` が `DecodeWorker<H>` のフィールドとしてまとまっている
- `run_worker` がジョブの dispatch のみになり、処理本体がメソッドに分離されている
- 既存のデコード・終端・flush の動作が変わらない (テスト・clippy が通る)

## 解決方法

`src/decode.rs` の `run_worker` を `DecodeWorker<H>` 構造体に移行した。

- `state` / `handler` / `pending_user_data` / `terminated` を `DecodeWorker<H>` のフィールドとしてまとめた
- 終端遷移を `enter_terminated` メソッド 1 箇所に集約した (従来 4 箇所の重複)
- ジョブ種別ごとの処理を `handle_decode` / `handle_flush` / `finish` メソッドに分離した
- `run_worker` free 関数を廃止し、エントリポイントは `DecodeWorker::run` 関連関数にした。`run` はジョブを各メソッドへ dispatch するだけになった
- `drain_frames` / `discard_queued_frames` を free 関数から `DecodeWorker` のメソッドに移し、3 引数の引き回しを解消した
- `DecoderState` の不要な `pub` (`decode` / `send_eos`) を外した
- チャネル破棄 (`Err(_)`) 時も `finish` で後始末して return する

動作は従来と同等。ビルド・clippy・fmt が通る。テストは CUDA 依存のため CI で確認する。
