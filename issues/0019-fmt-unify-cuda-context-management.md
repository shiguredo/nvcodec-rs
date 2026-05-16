# 0019-fmt-unify-cuda-context-management

Created: 2026-05-10
Model: DeepSeek v4-pro

## 背景

`EncoderState` のメソッド間で CUDA コンテキスト管理の責務が不整合。

`with_context` で自己完結しているメソッド:
- `reconfigure` (L680): `self.lib.clone().with_context(self.ctx, || self.reconfigure_inner(...))`
- `get_sequence_params` (L890): `self.lib.with_context(self.ctx, || self.get_sequence_params_inner())`
- `cleanup_buffer_pool` (L978): `self.lib.with_context(self.ctx, || { ... })`

`clone()` の有無は `with_context` が `&self` を取ることに起因する借用制約による: `reconfigure` は `&mut self` のため `self.lib.clone()` が必要だが、`get_sequence_params` と `cleanup_buffer_pool` は `&self` のため直接 `self.lib` を借用できる。いずれも意図的な差ではない。

呼び出し元任せでコンテキスト管理が行われていないメソッド:
- `encode_frame` (L1030): NVENC API 呼び出し（`cuMemcpyHtoD`, `nvEncMapInputResource`, `nvEncEncodePicture`）を含むがコンテキスト管理なし
- `lock_and_copy_bitstream` (L1081): `nvEncLockBitstream` / `nvEncUnlockBitstream` を含むがコンテキスト管理なし
- `send_eos` (L1132): `nvEncEncodePicture` を含むがコンテキスト管理なし
- `unmap_resource` (L1018): `nvEncUnmapInputResource` を含むがコンテキスト管理なし

これらのメソッドは `run_worker` と `drain_one_with_ctx` での手動 `cuCtxPushCurrent` / `cuCtxPopCurrent` に依存している。

## 問題箇所

`run_worker` 内の 3 箇所の手動 push/pop:

1. encode パス (L1395-1403): `encode_frame` の呼び出し前後
2. encode 失敗パス (L1417-1422): `unmap_resource` の呼び出し前後
3. terminate パス (L1442-1448): `send_eos` の呼び出し前後

`drain_one_with_ctx` 内の 2 箇所の手動 push/pop:

4. lock パス (L1473-1479): `lock_and_copy_bitstream` の呼び出し前後
5. unmap パス (L1502-1507): `unmap_resource` の呼び出し前後

## 問題

1. 手動 push/pop は漏れのリスクがある — `cuCtxPushCurrent` に成功して `cuCtxPopCurrent` が漏れた場合、CUDA コンテキストスタックが破壊される。当該リスクは issue 0012 の `ReleaseGuard` 導入により `with_context` 側では解決済みだが、手動 push/pop 箇所は `ReleaseGuard` の保護外。
2. `.expect()` パニックの温床 — 手動 push/pop 箇所では `cuCtxPushCurrent` の戻り値に `.expect()` を使っており、GPU リセット等でパニックする（issue 0016 として切出し済み。本 issue の完了により自然解決する）。

## 修正方針

### 基本方針

`EncoderState` の NVENC API を呼ぶ全メソッドが `with_context` で自己完結するように統一する。`run_worker` と `drain_one_with_ctx` から手動 push/pop をすべて除去する。

`drain_one_with_ctx` では `lock_and_copy_bitstream` と `unmap_resource` がそれぞれ `with_context` を呼ぶため push/pop が合計 2 回発生する。責務の一貫性を優先し 2 回の push/pop は許容する。

### `&mut self` メソッドの borrow 制約への対応

`with_context(&self, ...)` と `&mut self` メソッドは同時借用できない。既存の `reconfigure` と同様に、`_inner` ヘルパーメソッドに実装を移し、外側のラッパーで `self.lib.clone().with_context(...)` するパターンに統一する:

- `encode_frame(&mut self, ...)` → `encode_frame_inner(&mut self, ...)` に処理を移し、`encode_frame` は clone + with_context ラッパー
- `send_eos(&mut self)` → `send_eos_inner(&mut self)` に処理を移し、`send_eos` は clone + with_context ラッパー
- `unmap_resource(&mut self, ...)` → `unmap_resource_inner(&mut self, ...)` に処理を移し、`unmap_resource` は clone + with_context ラッパー

### 各メソッドの変更

#### `encode_frame` / `encode_frame_inner`

```rust
fn encode_frame(
    &mut self,
    bfr_idx: usize,
    data: &[u8],
    options: &EncodeOptions,
) -> Result<(), Error> {
    let lib = self.lib.clone();
    lib.with_context(self.ctx, || self.encode_frame_inner(bfr_idx, data, options))
}

fn encode_frame_inner(
    &mut self,
    bfr_idx: usize,
    data: &[u8],
    options: &EncodeOptions,
) -> Result<(), Error> {
    unsafe {
        // 現行の L1037-1077 の処理をここに移動
        // cu_memcpy_h_to_d → map_resource → nvEncEncodePicture
        //
        // nvEncEncodePicture がエラーを返した場合:
        //   map_resource で mapped resource を獲得済みなら unmap_resource_inner(bfr_idx) を呼んでから Err を返す
    }
}
```

- `map_resource` は `encode_frame_inner` 内部からのみ呼ばれる。`with_context` 内でコンテキストは既に有効なため `map_resource` 自体は context-unaware のままとする。
- `map_resource` の RAII ガード不足（issue 0021）は、`encode_frame_inner` のエラーパスで `unmap_resource_inner` を明示的に呼ぶことで対応する。RAII ガード方式への変更は issue 0021 に委ねる。
- `encode_frame` が `with_context` の push に失敗した場合（map_resource 未呼び出し）、`?` で `run_worker` にエラーが伝播する。この場合 mapped resource は存在しないため unmap 不要。

#### `lock_and_copy_bitstream`

```rust
fn lock_and_copy_bitstream(
    &self,
    bfr_idx: usize,
) -> Result<(Vec<u8>, u64, PictureType), Error> {
    self.lib.with_context(self.ctx, || {
        unsafe {
            // 現行の L1085-1128 をここに移動
        }
    })
}
```

- `&self` のため `clone()` 不要。
- 既存の `ReleaseGuard` による `nvEncUnlockBitstream` 保護はそのまま残す。
- `with_context` の `pop_guard`（`ReleaseGuard`）と `_unlock_guard` が二重の RAII ガードを形成する。正常パスでは `_unlock_guard` drop → `with_context` の明示的 pop の順で正しく動作する。panic 時も Drop 順序により unlock → pop の順が保証される。

#### `send_eos` / `send_eos_inner`

```rust
fn send_eos(&mut self) -> Result<(), Error> {
    let lib = self.lib.clone();
    lib.with_context(self.ctx, || self.send_eos_inner())
}

fn send_eos_inner(&mut self) -> Result<(), Error> {
    unsafe {
        // 現行の L1133-1146 をここに移動
    }
}
```

#### `unmap_resource` / `unmap_resource_inner`

```rust
fn unmap_resource(&mut self, bfr_idx: usize) {
    let lib = self.lib.clone();
    let _ = lib.with_context(self.ctx, || {
        self.unmap_resource_inner(bfr_idx);
        Ok(())
    });
}

fn unmap_resource_inner(&mut self, bfr_idx: usize) {
    unsafe {
        let Some(mapped) = self.mapped_inputs[bfr_idx].take() else {
            return;
        };
        let _ = self
            .encoder
            .nvEncUnmapInputResource
            .map(|f| f(self.h_encoder, mapped));
    }
}
```

- `unmap_resource_inner` は context-unaware な内部ヘルパー。`encode_frame_inner` のエラーパスから呼ばれる際、`encode_frame` の `with_context` 内で実行されるため二重 push/pop が発生しない。`cleanup_buffer_pool` の `with_context` 内からも同様に呼べる。
- `unmap_resource`（ラッパー）は `drain_one` から呼ばれる。`drain_one` の呼び出し元 `run_worker` にはコンテキストがないため、ラッパー経由で with_context が必要。

### `run_worker` の変更

以下の手動 push/pop ブロックを削除し、各メソッド呼び出しを直接行う:

1. encode パス (L1395-1403):
   ```rust
   // 変更前
   lib.cu_ctx_push_current(ctx).expect("...");
   let encode_result = state.encode_frame(bfr_idx, &data, &options);
   // pop
   ```
   ```rust
   // 変更後
   let encode_result = state.encode_frame(bfr_idx, &data, &options);
   ```

2. encode 失敗パス (L1417-1422):
   ```rust
   // 変更前
   let _ = lib.cu_ctx_push_current(ctx);
   state.unmap_resource(bfr_idx);
   let _ = lib.cu_ctx_pop_current(&mut popped);
   callback(Err(e));
   ```
   ```rust
   // 変更後（encode_frame_inner 内で unmap 済みのため不要）
   callback(Err(e));
   ```

3. terminate パス (L1442-1448):
   ```rust
   // 変更前
   lib.cu_ctx_push_current(ctx).expect("...");
   let _ = state.send_eos();
   // pop
   ```
   ```rust
   // 変更後
   let _ = state.send_eos();
   ```

### `drain_one_with_ctx` の変更 → `drain_one` にリネーム

2 箇所の手動 push/pop ブロック（lock 用, unmap 用）を削除し、各メソッド呼び出しを直接行う。関数名から `_with_ctx` を外して `drain_one` にリネームする:

```rust
// 変更前
fn drain_one_with_ctx<F, T>(...) {
    let lib = state.lib.clone();
    let ctx = state.ctx;
    lib.cu_ctx_push_current(ctx).expect("...");  // lock 用
    let lock_result = state.lock_and_copy_bitstream(bfr_idx);
    // pop
    // ...
    let lib = state.lib.clone();
    let ctx = state.ctx;
    let _ = lib.cu_ctx_push_current(ctx);  // unmap 用
    state.unmap_resource(bfr_idx);
    // pop
    state.i_got += 1;
}

// 変更後
fn drain_one<F, T>(...) {
    let bfr_idx = state.i_got % state.n_encoder_buffer;
    let lock_result = state.lock_and_copy_bitstream(bfr_idx);
    match lock_result { ... }
    state.unmap_resource(bfr_idx);  // unmap_resource が自身で with_context する
    state.i_got += 1;
}

`lock_and_copy_bitstream` と `unmap_resource` がそれぞれ `with_context` で自己完結するため、`drain_one` 側のコンテキスト管理は不要になる。push/pop が合計 2 回発生するが、基本方針で述べたとおり責務の一貫性を優先して許容する。

### 他の issue との関係

- **issue 0016** (`.expect()` パニック): 本 issue の完了により手動 push/pop が消滅するため自然解決する。
- **issue 0020** (drain `.expect()` 除去): `drain_one` の Ok/Err 分岐内のロジック変更のため、本 issue での構造変更と競合する。規約上は 0019 が先（番号が小さい）だが、実装上はどちらを先に適用してもアダプト可能。
- **issue 0018** (drain エラー握り潰し): 0020 に依存。0019/0020/0018 の適用順序で競合する場合は、0019 を先に適用し、0019 完了後のコードベースに対して 0020 と 0018 を再適用する方針とする。
- **issue 0021** (map_resource の RAII ガード): `encode_frame_inner` のエラーパスで `unmap_resource_inner` を明示的に呼ぶことで、mapped resource の unmap 漏れは解決する。RAII ガード方式への変更は issue 0021 に委ねる。
- **issue 0012** (with_context の panic 安全性): 既に解決済み。本 issue の修正は `with_context` の適用範囲を拡大するだけで、0012 で確立された panic 安全性をそのまま継承する。

## 変更対象ファイル

- `src/encode.rs`:
  - `encode_frame` (L1030-1079): `encode_frame` ラッパー + `encode_frame_inner` に分割。エラーパスで `unmap_resource_inner` 呼び出しを追加。
  - `lock_and_copy_bitstream` (L1081-1129): `with_context` でラップ。`ReleaseGuard` は維持。
  - `send_eos` (L1132-1148): `send_eos` ラッパー + `send_eos_inner` に分割。
  - `unmap_resource` (L1018-1028): `unmap_resource` ラッパー + `unmap_resource_inner` に分割。
  - `run_worker` (L1374-1458): 手動 push/pop の除去。encode 失敗パスから `unmap_resource` 呼び出しを削除。
  - `drain_one_with_ctx` (L1461-1510): 手動 push/pop の除去。`drain_one` にリネーム。呼び出し元 (`run_worker`) の関数名も更新。
- `CHANGES.md`: `[UPDATE]` エントリを `## develop` に追加。

## テスト戦略

本修正は内部実装のリファクタリングであり、新たな公開 API やロジックを追加しない。コンテキスト管理のタイミングが変わることによる副作用がないことを以下で確認する:

1. **既存 GPU テストの回帰確認**: `src/encode.rs` の `#[cfg(test)] mod tests` 内の既存エンコードテストをすべて実行し、正常系に回帰がないことを確認する。
2. **コードレビュー**: `encode_frame_inner` のエラーパスで mapped resource の unmap が漏れていないこと、`run_worker` の各分岐でコンテキスト不足による未定義動作が発生しないことを確認する。
3. **`with_context` push 失敗時**: `?` 演算子によりカスケードエラーとなり `run_worker` の既存エラーハンドリングに乗る。新たなエラーパスではないため特別なテストは不要。

## API 互換性

公開 API（`Encoder::encode`, `Encoder::reconfigure`, `Encoder::flush`, `Encoder::get_sequence_params`）への変更はない。`EncoderState` の private メソッドの内部実装変更のみ。正常系の振る舞い（エラー条件、戻り値の型、コールバック呼び出し順序）に影響しない。

## CHANGES.md 追記予定

```markdown
- [UPDATE] `EncoderState` の CUDA コンテキスト管理を `with_context` パターンに統一する
  - `encode_frame`, `lock_and_copy_bitstream`, `send_eos`, `unmap_resource` でコンテキスト管理を自己完結させる
  - `run_worker` と `drain_one_with_ctx` から手動の `cuCtxPushCurrent` / `cuCtxPopCurrent` を除去する
