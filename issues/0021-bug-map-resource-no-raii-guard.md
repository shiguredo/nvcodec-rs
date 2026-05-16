# 0021-bug-map-resource-no-raii-guard

Created: 2026-05-10
Model: DeepSeek v4-pro

## 背景

`encode_frame` 内で `map_resource` を呼ぶが、エラー発生時の unmap は呼び出し元 (`run_worker` のエラー分岐) に依存している。`lock_and_copy_bitstream` が `ReleaseGuard` でロック解除を確実に行っているのと対照的であり、設計上の脆弱性がある。

`decode.rs` L618 では `cuvid_unmap_video_frame` に対して `ReleaseGuard` による RAII パターンが適用されており、エンコーダ側だけが取り残されている。

## 問題箇所

```rust
// encode.rs:1052 — map_resource にガードがない
let mapped = self.map_resource(bfr_idx)?;
// encode_frame の残りの処理 (L1070-1075 の nvEncEncodePicture) で
// エラーが発生しても encode_frame 内では unmap されない
```

```rust
// encode.rs:1081-1129 — lock_and_copy_bitstream はガードあり（正しい）
let _unlock_guard = ReleaseGuard::new(move || {
    if let Some(f) = unlock_fn {
        let _ = f(h_encoder, output_bitstream);
    }
});
```

## 問題

`encode_frame` が `map_resource` (L1052) を呼んだ後、`nvEncEncodePicture` (L1070-1075) がエラーを返した場合、`encode_frame` 自体は unmap を行わない。現在は `run_worker` のエラー分岐 (L1415-1424) で手動 unmap しているため、現状のコードでは機能的に unmap 漏れは発生していない。

しかし、以下の理由で設計上の脆弱性がある:

1. **責務の分離違反**: `encode_frame` が map したリソースのクリーンアップ責務が、呼び出し元の `run_worker` に漏れ出している。これは `encode_frame` の実装詳細に依存しており、将来のリファクタリング時に容易に破綻する。
2. **呼び出し元増加リスク**: 将来的に `encode_frame` が別の文脈から呼ばれた場合、新たな呼び出し元が unmap を忘れる可能性がある。
3. **CUDA コンテキストへの暗黙の依存**: `encode_frame` 内のガードが Drop 時に `nvEncUnmapInputResource` を呼ぶには、呼び出し元が `cuCtxPushCurrent` で CUDA コンテキストをアクティブ化している必要がある。現状この前提は満たされているが、`encode_frame` 単体では保証されていない。

## 修正方針

`encode_frame` 内で、`map_resource` 成功後に `ReleaseGuard` を生成し、以下のパターンを適用する:

- **エラー時**: `ReleaseGuard` の Drop により自動的に `nvEncUnmapInputResource` が呼ばれ、`mapped_inputs[bfr_idx]` が `None` にクリアされる
- **成功時**: `ReleaseGuard::cancel()` を呼び、unmap は発生させない。リソースは引き続き mapped 状態を維持し、後続の `drain_one_with_ctx` が `unmap_resource` を担当する

`nvEncUnmapInputResource` の戻り値は既存の `unmap_resource` および `decode.rs` のガードと同様に `let _ =` で捨てる（NVENC API の unmap 失敗は回復不能であり、できることはない）。

### コードパターン

```rust
fn encode_frame(
    &mut self,
    bfr_idx: usize,
    data: &[u8],
    options: &EncodeOptions,
) -> Result<(), Error> {
    unsafe {
        let expected_size = self.buffer_format_enum.frame_size(self.width, self.height)?;
        if data.len() != expected_size {
            return Err(Error::new_custom("encode", "invalid frame data size"));
        }

        self.lib.cu_memcpy_h_to_d(
            self.device_inputs[bfr_idx],
            data.as_ptr().cast(),
            data.len(),
        )?;

        // リソースマップ
        let mapped = self.map_resource(bfr_idx)?;

        // エラー時に自動で unmap するガード
        let mapped_entry = &mut self.mapped_inputs[bfr_idx];
        let unmap_fn = self.encoder.nvEncUnmapInputResource;
        let h_encoder = self.h_encoder;
        let unmap_guard = ReleaseGuard::new(move || {
            *mapped_entry = None;
            if let Some(f) = unmap_fn {
                let _ = f(h_encoder, mapped);
            }
        });

        // エンコードパラメータ構築（self.* の読み取りアクセス、disjoint borrow により問題なし）
        let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
        pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
        pic_params.inputWidth = self.width;
        pic_params.inputHeight = self.height;
        pic_params.inputPitch = self.pitch;
        pic_params.inputBuffer = mapped;
        pic_params.outputBitstream = self.bitstream_buffers[bfr_idx];
        pic_params.bufferFmt = self.buffer_format;
        pic_params.pictureStruct = sys::_NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME;
        pic_params.inputTimeStamp = self.frame_count * self.framerate_den;
        pic_params.encodePicFlags = options.to_pic_flags();
        pic_params.frameIdx = self.i_to_send as u32;

        // frame_count の加算は現位置のまま（disjoint borrow によりガード存命中でも可能）
        self.frame_count += 1;

        // nvEncEncodePicture 呼び出し
        let status = self
            .encoder
            .nvEncEncodePicture
            .map(|f| f(self.h_encoder, &mut pic_params))
            .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
        Error::check_nvenc(status, "nvEncEncodePicture")?;

        // 成功: ガードをキャンセル（mapped 状態を維持）
        unmap_guard.cancel();

        Ok(())
    }
}
```

## 修正後のエラーパス検証

| # | 状況 | 修正前 | 修正後 |
|---|------|--------|--------|
| 1 | map 成功 → encode 失敗 | `run_worker` が手動 unmap（脆弱） | ガードの Drop が自動 unmap |
| 2 | map 成功 → encode 成功 | drain が unmap（変更なし） | ガード cancel → drain が unmap（変更なし） |
| 3 | map 前にエラー（データサイズ不一致等） | `run_worker` が空の push/pop（無害） | ガード生成前に return するため処理なし |

## 変更対象

**`encode_frame`** (encode.rs L1030-1079):
- `map_resource` 呼び出し直後に `ReleaseGuard` を生成する
- `encode_frame` 成功時に `guard.cancel()` を呼ぶ
- `self.frame_count += 1` は現位置 (L1068) のまま維持する（エラーが発生していた場合も加算済み、という現在の挙動を変更しない）

`run_worker` のエラー分岐からの `unmap_resource` 削除は issue 0019 が担当するため、本 issue のスコープ外である。

## テスト戦略

**単体テスト** (`src/encode.rs` 内の `#[cfg(test)] mod tests`):
- NVENC API はハードウェア依存であり、`nvEncEncodePicture` の意図的なエラー注入が困難
- 以下の方法で検証する:
  1. 既存の `test_encode_*_black_frame` テストで正常系の回帰確認を実施する
  2. `ReleaseGuard` の Drop 挙動のテストを `src/lib.rs` のテストとして追加する: `ReleaseGuard::new(f)` の `f` が `Drop` 時に正しく呼ばれること、および `cancel()` 後は呼ばれないことを検証する

## 関連 Issue

- `issues/0019-fmt-unify-cuda-context-management.md`: `encode_frame` を `encode_frame_inner` に分割し、CUDA コンテキスト管理を統一する計画がある。本 issue は 0019 完了後に着手し、0019 が導入する `encode_frame_inner` に対して本修正を適用する。
- `issues/0018-bug-drain-error-silently-dropped.md`: `drain_one_with_ctx` のエラー分岐の修正。適用順序に注意すること。
- `issues/0020-fmt-remove-expect-from-drain.md`: `drain_one_with_ctx` の `expect` 除去。同上。

## 備考

- `EncoderState::Drop` から呼ばれる `cleanup_buffer_pool` (L977-998) が全 mapped resource を走査して unmap するため、プロセス終了時には最終的に回収される。本 issue が対処するのは通常運用中の長時間リーク（ワーカースレッド生存期間中の累積）である
- 変更後は `CHANGES.md` の `## develop` セクションに以下を追記する:
  - `- [FIX] encode_frame 内で map_resource の後始末に ReleaseGuard を使用し、エラー発生時に自動で unmap されるようにする`
  - `  - @melpon`
