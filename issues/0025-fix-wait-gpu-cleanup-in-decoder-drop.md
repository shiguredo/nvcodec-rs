# 0025-fix-wait-gpu-cleanup-in-decoder-drop

- Created: 2026-08-05
- Completed: 2026-08-12
- Branch: feature/fix-wait-gpu-cleanup-in-decoder-drop
- Polished: 2026-08-05

## 目的

利用者が `Decoder` を短時間に drop → 新規 `Decoder::new` を繰り返す運用で、H.265 ストリームの 2 〜 3 回目の再作成後に `pfnSequenceCallback` 内の `cuvidCreateDecoder()` が `status=1` で失敗する現象を軽減する。

**「軽減」であって「完全解決」ではない**。前 `Decoder` の GPU 側 cleanup を明示的に待つことで再作成失敗のレースウィンドウを縮めるが、`cuvidDestroyDecoder` の内部処理が本当に `cuCtxSynchronize` で待てるかは NVDEC SDK doc で明示されておらず、実測ベースの軽減策として扱う。

## 現状

`src/decode.rs` の `impl Drop for DecoderState` の `drop` メソッドは `cuvid_destroy_decoder` を呼んだ後に GPU 側の同期を取らずに次の後片付け (`cuvid_ctx_lock_destroy` / `cu_ctx_destroy`) に進む。このため呼び出し側スレッドは `cuvidDestroyDecoder` の完了を GPU 側で待たずに Drop から抜ける。

利用者は公開型 `Decoder<H>` を drop する形で `DecoderState` の Drop を誘発するのが通常の経路。この経路では `impl<H> Drop for Decoder<H>` が worker を終了させ、`run_worker` の Terminate 分岐が `send_eos()` を呼ぶ。`send_eos()` は内部で `cu_ctx_synchronize` を 1 度呼ぶが、この後に `DecoderState::drop` が走って `cuvid_destroy_decoder` が呼ばれる。**`cuvid_destroy_decoder` 自身が GPU 側に非同期 cleanup を残す挙動と推定される** (下記の再現例が示す)。

standalone (`Decoder` を直接呼び出す短いループ) で下記が再現する。

- H.265 のストリームを decode → 途中で `Decoder` を drop → 新規 `Decoder::new` を短時間で繰り返すと、2 〜 3 回目の再作成後の `pfnSequenceCallback` 内で `cuvidCreateDecoder()` が `status=1` で失敗する
- H.264 の同じ操作は standalone では成功する (機序は不明。H.265 側の GPU 上メモリ配置の差など codec 依存の要因と推定されるが本 issue では追わない)

推定原因は、前 `Decoder` の `cuvidDestroyDecoder` 呼び出し後に GPU 側のリソース cleanup が非同期に進む間に、新 `Decoder` の `cuvidCreateDecoder` が同一 CUDA driver 上で走り、GPU リソース競合を起こすこと。SDK doc に明示的な裏付けはないが、`cuCtxSynchronize` (「Blocks for a context's tasks to complete」) で cleanup 完了を待たせることで再現しなくなることを実測で確認する方針。

## 設計方針

`impl Drop for DecoderState` の `drop` メソッド内で `cuvid_destroy_decoder` を呼んだ直後、後片付けに進む前に、同じ `with_context` の中で `cu_ctx_synchronize` を呼んで GPU 側の cleanup 完了を待つ。

### 呼び出し形式

現行の (既存の `if !self.decoder.is_null()` null チェックガード内の) 以下を:

```rust
let _ = self
    .lib
    .with_context(self.ctx, || self.lib.cuvid_destroy_decoder(self.decoder));
```

以下のようにクロージャ内で 2 つの操作を並べる形に変更する (null チェックガードは維持):

```rust
let _ = self.lib.with_context(self.ctx, || {
    let _ = self.lib.cuvid_destroy_decoder(self.decoder);
    self.lib.cu_ctx_synchronize()
});
```

`cuvid_destroy_decoder` が失敗しても `cu_ctx_synchronize` を必ず呼ぶ (destroy が失敗した状態こそ GPU 側 cleanup の完了を待ちたい場面)。`cu_ctx_synchronize` の戻り値は Drop 内でエラー伝播できないため、既存の他の cleanup 呼び出しと同じく `let _ =` で戻り値の `Result` を捨てる。

### 位置と範囲

同期は `cuvid_destroy_decoder` の直後 (現行 Drop の他の 3 段階、`cuvid_destroy_video_parser` / `cuvid_ctx_lock_destroy` / `cu_ctx_destroy` の間ではなく) に置く。`cuCtxSynchronize` は current context の全 pending work を待つため、`cuvid_destroy_decoder` 直後に呼べば Parser 由来の pending work も含めて待てる (Parser 破棄の後などに別途 synchronize を置く必要はない)。

### 本 issue の変更が有効なシナリオ / 有効でないシナリオ

- **有効**: 利用者が `Decoder<H>` を drop して短時間で新規 `Decoder::new` を呼ぶシナリオ (本 issue の再現ケース)
- **有効でない**: `handle_video_sequence_inner` 内での destroy+create 経路 (現行の destroy → create、および issue 0024 の `max_coded_*` = `None` の fallback、`max_coded_*` = `Some` かつ codec / chroma / bit depth / progressive 変化時の destroy+create)。これらは `DecoderState::drop` を経由せず `handle_video_sequence_inner` 内で直接 `cuvid_destroy_decoder` → `cuvid_create_decoder` を呼ぶため、本 issue の Drop 内 synchronize は効かない

`handle_video_sequence_inner` 内の destroy+create 経路への同種対応は、必要になれば別 issue で扱う (本 issue の scope 外)。

### コスト

追加コストは `Decoder` の drop 時のみ。`DecoderState` は Decoder 専用の CUDA context を保持する (`src/decode.rs` の `DecoderState::new_with_codec` で `cu_ctx_create` により生成) ため、`cuCtxSynchronize` が他リソースの pending work を巻き添えにする副作用は発生しない。

## 完了条件

- `impl Drop for DecoderState` の `drop` メソッド内で `cuvid_destroy_decoder` の直後 (同じ `with_context` クロージャ内、`cuvid_destroy_decoder` の失敗有無に関わらず) に `cu_ctx_synchronize` が呼ばれる
- `Decoder` を drop → `Decoder::new` を短時間で繰り返す手動再現手順 (実 GPU 環境で H.265 のストリームを流す) で、修正前は 2 〜 3 回目の再作成後の `cuvidCreateDecoder failed` が発生し、修正後は同じ手順で発生しなくなることが確認できる
- `CHANGES.md` に `[FIX]` エントリが追加されている (種別順で既存の `[CHANGE]` エントリ群の後ろに配置)

自動テストの追加は本 issue の scope 外とする (GPU 依存のため mock は使えず、CI 環境で安定に再現できないため。issue 0017 pending と同様の判断)。

## 解決方法

本 issue はコード変更を行わず、対応不要として closed にする。

現行の `Decoder<H>` の Drop 経路は、worker に終了を要求して終了を待つ。worker は終了前に `DecoderState::send_eos` を呼び、EOS を送信してから `cuCtxSynchronize` ですべてのデコード処理の完了を待つ。その後、`DecoderState` の Drop で parser、decoder、context lock、CUDA context の順に破棄する。

NVIDIA の NVDEC Video Decoder API Programming Guide は、デコード完了後に `cuvidDestroyDecoder` で decoder session と割り当て済みリソースを解放し、その後 CUDA context を破棄するライフサイクルを定めている。`cuvidDestroyDecoder` の後に `cuCtxSynchronize` を追加する要件は示していない。また、CUDA Driver API の `cuCtxSynchronize` が待つ対象は current context で先行して要求された task であり、`cuvidDestroyDecoder` 内部の cleanup がこの対象になることは規定されていない。

- [NVDEC Video Decoder API Programming Guide 13.0](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvdec-video-decoder-api-prog-guide/index.html)
- [CUDA Driver API: Context Management](https://docs.nvidia.com/cuda/archive/13.0.2/cuda-driver-api/group__CUDA__CTX.html)

報告された `status=1` は `CUDA_ERROR_INVALID_VALUE` であり、それだけでは GPU リソース競合や decoder cleanup の未完了を示さない。したがって、失敗原因を `cuvidDestroyDecoder` 後の非同期 cleanup とする根拠はなく、提案していた同期処理の有効性も仕様から確認できない。

再現確認のために作成した `test_decoder_recreate_h265_repeatedly` にも、次の観測漏れがあった。

- `Decoder::decode` は worker への job 送信成功を返すだけで、実際のデコード結果を返さない
- `cuvidCreateDecoder` の失敗はコールバック経由で通知されるが、テストは受信側から結果を取得していない
- 利用者が `flush` を呼ばずに drop しても、`Decoder<H>` の Drop 経路は内部で EOS 送信と `cuCtxSynchronize` を実行する

このため、修正前の 2 回の CI 成功は現象が再現しなかったことを証明しない。一方、失敗を観測するようにテストを修正しても、CI 環境で修正前の失敗を安定して再現できなければ修正効果を検証できない。根拠と検証手段がないまま同期処理を追加することは避け、再現テストも採用しない。コード変更がないため `CHANGES.md` も変更しない。

今後、同じ現象が再現した場合は、GPU、NVIDIA driver、Video Codec SDK の各バージョン、入力ストリーム、完全なエラー情報、最小再現コードを記録する。修正前後を同一環境で比較できる状態になった場合に、本 issue を reopened にして原因と対策を改めて検討する。

## 関連 issue

- 0017 (pending): `handle_video_sequence_inner` の destroy-then-create 順序による復旧不能問題。本 issue とは対象箇所が異なる (0017 は `handle_video_sequence_inner` 内、本 issue は `DecoderState::drop`)
- 0024 (open): `cuvidReconfigureDecoder` への切替の根本解。本 issue とは対象箇所が異なる (0024 は `handle_video_sequence_inner` 内、本 issue は `DecoderState::drop`)。0024 の Step 4 / Step 5 の destroy+create 経路は `handle_video_sequence_inner` 内で発生し `DecoderState::drop` を経由しないため、本 issue の Drop 内 synchronize は効かない
