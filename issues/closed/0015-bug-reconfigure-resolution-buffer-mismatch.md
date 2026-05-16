# 0015-bug-reconfigure-resolution-buffer-mismatch

Created: 2026-05-10
Completed: 2026-05-16
Branch: feature/fix-reconfigure-buffer-mismatch
Model: deepseek-v4-pro

## 優先度

P0 — GPU メモリの範囲外書き込みにより Undefined Behavior に至る。

## 背景

Issue 0007（エンコーダーの動的解像度変更に対応する）の完了内容は、`ReconfigureParams` に `width` / `height` フィールドを追加し `reconfigure_inner` で `self.width` / `self.height` / `self.expected_frame_size` を更新するものであった。しかし以下の値は更新されず残るため、解像度変更後のエンコードは破綻する:

1. **`self.pitch`** — 初期化時の解像度で計算された値がそのまま残る。`reconfigure_inner` は pitch を再計算しない。
2. **バッファプール** — `device_inputs`、`registered_resources`、`bitstream_buffers` は初期解像度の `frame_size()` で割り当てられたまま。`registered_resources` 登録時の `width` / `height` / `pitch` 値も旧値のまま。

## 問題

### 解像度拡大時のバッファオーバーフロー

`encode_frame` のフロー:
1. `self.expected_frame_size` は `reconfigure_inner` で新解像度に更新済みのため、`data.len() != expected_size` のチェックを通過する
2. `cu_memcpy_h_to_d(self.device_inputs[bfr_idx], data, data.len())` が実行されるが、`device_inputs[bfr_idx]` は初期解像度の `frame_size` で割り当てられたまま
3. 新解像度 > 旧解像度の場合、`data.len() > frame_size(initial)` となり GPU メモリの範囲外書き込み（Undefined Behavior）が発生する

### 解像度縮小時の pitch 不整合

`pic_params.inputPitch = self.pitch` は初期解像度の pitch（例: 1920）のまま。縮小時（例: 1280）の正しい pitch は `bytes_per_row(1280)` だが、`self.pitch` は 1920 のまま NVENC に渡される。これにより NVENC が入力バッファの各行の開始位置を誤解釈し、破損したビットストリームが出力される。

### 解像度縮小時の GPU メモリ浪費

縮小時はバッファオーバーフローこそ発生しないが、`device_inputs` が旧（大）サイズのまま割り当てられ続け、GPU メモリを浪費する。

### パイプライン中の reconfigure との競合

`run_worker` の `Job::Reconfigure` 分岐は、`i_to_send != i_got`（in-flight フレームが存在する）状態でも `reconfigure` を即座に実行する。この状態でバッファプールを再構築すると、後続の `drain_one_with_ctx` が解放済みの `registered_resources` ハンドルを使用し use-after-free に至る。

## 再現手順

**前提条件**: `max_encode_width` / `max_encode_height` を初期解像度より大きな値で指定すること。指定しない場合、`initialize_encoder` 内で `width` / `height` と同じ値が設定されるため、`reconfigure_inner` のバリデーションでエラーが返り再現に至らない。

1. `EncoderConfig { width: 640, height: 480, max_encode_width: Some(1280), max_encode_height: Some(720), .. }` でエンコーダーを作成する
2. `reconfigure(ReconfigureParams { width: Some(1280), height: Some(720), ..Default::default() })` を呼ぶ
3. NV12 形式の 1280x720 フレームをエンコードする
4. AddressSanitizer 有効時は heap-buffer-overflow が検出される。ASan 無効時は GPU メモリ破壊により出力ビットストリームが不正になる（緑画面、デコード不能、またはクラッシュ）

## 変更対象ファイル

- `src/encode.rs` — `reconfigure_inner`、`cleanup_buffer_pool`、`init_buffer_pool`、`run_worker`
- `CHANGES.md` — `[FIX]` エントリを `## develop` に追加

## 推奨対応

### 基本方針

バッファプール全体の再構築を実装し、解像度変更を正しく動作させる。

### 1. `cleanup_buffer_pool` を idempotent にし、ベクタをクリアする

`cleanup_buffer_pool` を再呼び出し可能にする:

```rust
fn cleanup_buffer_pool(&mut self) {
    if self.device_inputs.is_empty() {
        return; // 既にクリーンアップ済み
    }
    // 以下、既存の n_encoder_buffer ループ
    // ...
    self.device_inputs.clear();
    self.registered_resources.clear();
    self.bitstream_buffers.clear();
    self.mapped_inputs.fill(None);
}
```

これにより:
- `init_buffer_pool` 失敗後に `Drop` から `cleanup_buffer_pool` が再呼び出しされてもパニックしない
- `n_encoder_buffer` による全リソース列挙の責務を維持する
- `mapped_inputs.fill(None)` で全エントリを明示的にリセットする

### 2. `reconfigure_inner` で `self.pitch` を更新する

解像度変更の分岐内で `self.pitch = self.buffer_format_enum.bytes_per_row(self.width)?;` を追加する。pitch は幅のみに依存するため、`params.width.is_some()` が true の場合に更新すればよい。`params.height` のみの変更では pitch は不変であるため更新不要。

### 3. `reconfigure_inner` でバッファプールを再構築する

解像度変更の分岐（`params.width.is_some() || params.height.is_some()`）内で、`self.cleanup_buffer_pool()` → `self.init_buffer_pool()` の順で呼ぶ。

- `cleanup_buffer_pool` は内部で `with_context` を使うが、`reconfigure_inner` の外側でも `with_context` が使われており二重になる。CUDA の `cuCtxPushCurrent` / `cuCtxPopCurrent` は参照カウント方式のため二重ネストは安全。
- クリーンアップ時のエラーは既存の仕様どおり `let _ =` で無視する。

### 4. `run_worker` の `Job::Reconfigure` 分岐で全 in-flight フレームを drain してから `reconfigure` を実行する

`state.i_got < state.i_to_send` の間 `drain_one_with_ctx` を呼ぶ。`Flush` 分岐と同様のループ構造にする。

- drain 中に callback が呼ばれる。callback から同一 `Encoder` の `reconfigure()` を呼ぶと、ワーカースレッドが `Job::Reconfigure` の処理中であるため応答できずデッドロックする。これは既存の `Flush` 分岐でも同様に存在する設計上の制約であり、本 issue の修正範囲外とする。
- `drain_one_with_ctx` 内の `.expect()` パニックが drain ループ追加により発現確率が上がる。Issue 0016 を先に解決することを推奨する（必須ではない）。

### 5. 解像度変更後の IDR フレーム送出（呼び出し元の責任）

`NvEncReconfigureEncoder` で解像度を変更した直後の最初のエンコードフレームには、呼び出し元が `EncodeOptions { force_idr: true, output_spspps: true, .. }` を指定する必要がある。これを怠ると新しい解像度の SPS/PPS がビットストリームに出力されず、デコーダーが再生不能になる。本 issue では `Encoder::reconfigure` の doc コメントに警告を追記する。コード上の自動強制は行わない（呼び出し元の責任）。

## 後方互換

`ReconfigureParams` の公開 API に変更なし。種別は `[FIX]`。

## テスト戦略

- **単体テスト**（`src/encode.rs` の `#[cfg(test)] mod tests`）:
  - 解像度拡大（640x480 → 1280x720）→ エンコードしたフレームが正しく出力されることを確認する
  - 解像度縮小（1280x720 → 640x480）→ 同様に確認する
  - `width` のみ / `height` のみの片方変更も確認する
  - エンコード中に `reconfigure` を発行し、全フレームが正しく完了することを確認する（パイプライン競合のテスト）
  - テスト用の `EncoderConfig` には `max_encode_width` / `max_encode_height` を明示的に大きな値で指定すること
- PBT は不要（単体テストで境界値をカバーできるため）

## 解決方法

`src/encode.rs` に以下の修正を行った:

1. **`cleanup_buffer_pool` を idempotent にする**: `device_inputs` が空の場合は早期リターンし、クリーンアップ後に全ベクタを `clear()` し `mapped_inputs` を `fill(None)` でリセットするようにした。これにより `Drop` からの再呼び出しや `reconfigure_inner` 内での再構築が安全になる。

2. **`reconfigure_inner` で `self.pitch` を更新**: 幅が変更された場合に `self.pitch = self.buffer_format_enum.bytes_per_row(self.width)?` を実行し、NVENC に正しい pitch 値が渡されるようにした。

3. **`reconfigure_inner` でバッファプールを再構築**: 解像度変更時（`params.width.is_some() || params.height.is_some()`）に `self.cleanup_buffer_pool()` → `self.init_buffer_pool()` を呼び、新しい解像度に対応する GPU メモリバッファを再割り当てするようにした。

4. **`run_worker` の `Job::Reconfigure` 分岐で in-flight フレームを drain**: `state.i_got < state.i_to_send` の間 `drain_one_with_ctx` を呼び、全 in-flight フレームを完了させてから `reconfigure` を実行するようにした。これによりバッファプール再構築と後続の `drain_one_with_ctx` の競合（use-after-free）を防ぐ。

5. **`Encoder::reconfigure` の doc コメントに警告を追加**: 解像度変更後の最初のエンコードフレームには `force_idr: true, output_spspps: true` を指定する必要があることを明記した。

### 追加したテスト

- `test_reconfigure_resolution_upscale_h264`: 640x480 → 1280x720 への解像度拡大
- `test_reconfigure_resolution_downscale_h264`: 1280x720 → 640x480 への解像度縮小
- `test_reconfigure_width_only_h264`: 幅のみの変更（640→960）
- `test_reconfigure_height_only_h264`: 高さのみの変更（480→720）
- `test_reconfigure_during_encoding_h264`: in-flight フレームが存在する状態での reconfigure（パイプライン競合の検証）
- テスト用の `test_encoder_config_with_max_resolution` および `create_black_frame` ヘルパー関数を追加
