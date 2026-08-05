# 0025-fix-wait-gpu-cleanup-in-decoder-drop

- Created: 2026-08-05
- Branch: feature/fix-wait-gpu-cleanup-in-decoder-drop

## 目的

`Decoder` を短時間に drop → 新規 `Decoder::new` を繰り返す運用で、H.265 ストリームの 2 〜 3 回目の再作成後に `pfnSequenceCallback` 内の `cuvidCreateDecoder()` が `status=1` で失敗する現象を軽減する。

## 現状

`src/decode.rs` の `DecoderState::Drop` は `cuvid_destroy_decoder` を呼んだ後に GPU 側の同期を取らずに次の後片付け (`cuvid_ctx_lock_destroy` / `cu_ctx_destroy`) に進む。このため呼び出し側スレッドは `cuvidDestroyDecoder` の完了を GPU 側で待たずに Drop から抜ける。

standalone (`Decoder` を直接呼び出す短いループ) で下記が再現する。

- H.265 のストリームを decode → 途中で `Decoder` を drop → 新規 `Decoder::new` を短時間で繰り返すと、2 〜 3 回目の再作成後の `pfnSequenceCallback` 内で `cuvidCreateDecoder()` が `status=1` で失敗する
- H.264 の同じ操作は standalone では成功する (codec 差)

推定原因は、前 `Decoder` の `cuvidDestroyDecoder` 呼び出し後に GPU 側のリソース cleanup が非同期に進む間に、新 `Decoder` の `cuvidCreateDecoder` が同一 CUDA driver 上で走り、GPU リソース競合を起こすこと。

## 設計方針

`DecoderState::Drop` で `cuvid_destroy_decoder` を呼んだ直後、後片付けに進む前に、同じ `with_context` の中で `cu_ctx_synchronize` を呼んで GPU 側の cleanup 完了を待つ。

- `cu_ctx_synchronize` は `CudaLibrary::load` で存在チェック済み (`src/lib.rs`)
- 追加コストは `Decoder` の drop 時のみで、通常運用 (Decoder を長時間保持する) には影響が小さい
- `with_context` は既に `cuvid_destroy_decoder` 呼び出しに使われているので、同じスコープで `cu_ctx_synchronize` を続けて呼べば context の push/pop を追加せずに済む (`cuCtxSynchronize` は current context に対して働くため、`with_context` の中で行う必要がある)

## 完了条件

- `DecoderState::Drop` で `cuvid_destroy_decoder` の直後に `cu_ctx_synchronize` が呼ばれる
- `Decoder` を drop → `Decoder::new` を短時間で繰り返す standalone テストが H.265 のストリームで `cuvidCreateDecoder failed` にならず完走することが確認できる
- `CHANGES.md` に `[FIX]` エントリが追加されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — `DecoderState::Drop` に `cu_ctx_synchronize` 呼び出しを追加
- `CHANGES.md` — `[FIX]` エントリを追加。文例:
  - `- [FIX] Decoder の drop 直後に新規 Decoder を作成すると H.265 で cuvidCreateDecoder が失敗する場合があるため、DecoderState::Drop で cuvidDestroyDecoder の後に cu_ctx_synchronize を呼んで GPU cleanup 完了を待つ`
  - `  - @担当者`

## 関連 issue

- 0017 (pending): destroy-then-create 順序による復旧不能問題とは独立 (順序ではなく GPU 側 cleanup タイミングの問題)
- 0024 (open): `cuvidReconfigureDecoder` への切替の根本解と併存させる
  - 0024 の `max_coded_*` = `Some` の reconfigure 経路では Decoder の destroy 自体が起きないため本 issue の影響なし
  - 0024 の `max_coded_*` = `None` の fallback 経路と、利用者が自前で `Decoder` を作り直すシナリオで本 issue の同期待ちが有効
