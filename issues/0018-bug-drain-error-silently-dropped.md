# 0018-bug-drain-error-silently-dropped

Created: 2026-05-10
Model: deepseek-v4-pro

## 背景

`drain_one_with_ctx` で `lock_and_copy_bitstream` がエラーを返した場合、`pending_user_data.pop_front()` が `None` を返すとコールバックが呼ばれず、エラーが通知されない。加えて `i_got += 1` がエラーにかかわらず無条件に実行されるため、後続の drain でバッファインデックスと `pending_user_data` の不整合が拡大する。

## 問題箇所

該当コード (`src/encode.rs`):

```rust
// encode.rs:1481-1509 (抜粋)
match lock_result {
    Ok((data, timestamp, picture_type)) => {
        let user_data = pending_user_data
            .pop_front()
            .expect("pending_user_data must not be empty during drain");
        callback(Ok(EncodedFrame { ... }));
    }
    Err(e) => {
        if let Some(_user_data) = pending_user_data.pop_front() {
            callback(Err(e));
        }
        // pending_user_data が空の場合、エラー通知なしでスルーされる
    }
}
// ...
state.i_got += 1;  // L1509: エラーにかかわらず無条件にインクリメント
```

## 問題

`pending_user_data` の長さと `i_to_send - i_got` の不変条件が正常系では常に成立する。しかし `lock_and_copy_bitstream` 自体の失敗（`nvEncLockBitstream` API エラーや `bitstreamBufferPtr` null）により Err パスに入った場合:

1. `if let Some(_user_data)` ガードにより、`pending_user_data` が空だと `callback(Err(e))` が呼ばれない
2. `i_got += 1` が無条件に実行されるため、`pending_user_data` の先頭要素が消費されずにインデックスだけが進む
3. 後続の drain 呼び出しでは `bfr_idx = i_got % n_encoder_buffer` が進む一方で `pending_user_data.pop_front()` は 1 つ前の user_data を返すため、**誤った user_data が後続フレームに紐付く**
4. 誤ったバッファインデックスに対して `lock_and_copy_bitstream` と `unmap_resource` が実行され、CUDA レベルの未定義動作（未初期化バッファ読み取り等）に発展する可能性がある

## 修正方針

`Err(e)` 分岐の `if let` ガードを外し、**`pending_user_data` の状態によらず必ず `callback(Err(e))` を呼ぶ**。またデコーダ側 `drain_frames`（decode.rs:794-798）と同様に `pending_user_data.clear()` で残存する user_data を破棄する。

- `i_got += 1` は**そのまま維持する**。`lock_and_copy_bitstream` で当該バッファのロック・アンロックは完了しており、後続で `unmap_resource` されるため、バッファインデックスは進める必要がある
- エラー種別は `lock_and_copy_bitstream` 由来の本来のエラー `e` を通知する（`missing user data` で上書きしない）
- `callback(Err(e))` を呼んだ後も `run_worker` の while ループは継続する（`i_got` は進み続け、残り全バッファの `lock_and_copy_bitstream` → `unmap_resource` が実行される）。`pending_user_data.clear()` により後続の drain では `pop_front()` が必ず `None` を返す。このとき:
  - Ok パス（`lock_and_copy_bitstream` 成功）では `pop_front()` が `None` を返す
  - **issue 0020 の修正（Ok パスの `expect()` → `callback(Err(...))` 化）が先に適用されていることが前提となる**。0020 未適用の場合、Ok パスの `.expect()` でパニックする
  - 0020 適用後は、残り全フレームの drain で `missing user data` エラーがコールバック通知される。この連続エラーは許容する（デコーダ側と異なり break しないのは、エンコーダのバッファプールリソースをすべて解放する必要があるため）
- `lock_and_copy_bitstream` の戻り後に `_unlock_guard`（`ReleaseGuard`）は drop 済みであり、`nvEncUnlockBitstream` は既に呼ばれている。その後に `callback(Err(e))` が呼ばれるため、ロック解除 → コールバックの順序は正しい

修正後コード（変更部のみ）:

```rust
Err(e) => {
    pending_user_data.clear();
    callback(Err(e));
}
```

## 変更対象ファイル

- `src/encode.rs` — `drain_one_with_ctx` の `Err(e)` マッチアーム（L1494-1498）の修正
- `CHANGES.md` — `[FIX]` エントリを `## develop` に追加

## CHANGES.md 追記予定

```markdown
- [FIX] `drain_one_with_ctx` で `lock_and_copy_bitstream` エラー時にコールバックが呼ばれない問題を修正する
  - `pending_user_data` が空でも必ずエラーを通知し、残存する user_data を破棄する
  - @担当者
```

## テスト戦略

`drain_one_with_ctx` は private 関数であり、`EncoderState` の内部状態と GPU ハードウェアに強く依存する。単体テストでのエラーパス再現には構造的な制約がある:

- `lock_and_copy_bitstream` のエラーを発生させるには、`nvEncLockBitstream` の関数ポインタ不在、または同 API のエラー戻り値が必要だが、これらをテストコードから注入する機構は現状存在しない
- `run_worker` は OS スレッド内で動作し、`EncoderState` を move で消費するため、外部から内部状態を書き換えられない

そのため本修正の検証は以下で行う:

1. **コードレビュー**: 修正内容が `Err(e)` 分岐の全パス（`lock_and_copy_bitstream` の 2 種類のエラー: `nvEncLockBitstream` API 失敗、`bitstreamBufferPtr` null）で正しく動作することをレビューで確認する
2. **既存 GPU テスト**: `src/encode.rs` の `#[cfg(test)] mod tests` 内の既存エンコードテストを実行し、正常系に回帰がないことを確認する
3. **手動テスト**: 実 GPU 上で `h_encoder` を null にする等の異常状態を人為的に作り、エラーコールバックが呼ばれることを確認する

## API 互換性

公開 API に変更なし。Err パスでのエラー通知の追加はバグ修正であり、正常系の動作に影響しない。

## 前提条件

**issue 0020 が先に適用されていること**。0020 未適用の状態で本修正のみを適用すると、Ok パスの `.expect()` でパニックが発生する（詳細は「修正方針」セクション参照）。
