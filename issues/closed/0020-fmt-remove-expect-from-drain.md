# 0020-fmt-remove-expect-from-drain

Completed: 2026-05-16
Branch: feature/fix-drain-error-handling
Created: 2026-05-10
Model: deepseek-v4-pro

> **注意**: この issue の実質的なカテゴリは `bug`（バグ修正）である。ファイル名の `fmt` は不適切であり、 issue 解決時に `0020-bug-remove-expect-from-drain.md` へ git mv すること。

## 優先度

P1 — ワーカースレッドのパニックにより CUDA リソースがリークする。

## 背景

`drain_one_with_ctx` の Ok パスで `pending_user_data.pop_front().expect(...)` を使っている。

## 再現手順

`pending_user_data` の長さと `i_to_send - i_got` の不変条件は正常系では常に成立する。不変条件が破れるケースとして以下が考えられる:

1. `lock_and_copy_bitstream` がエラーを返し Err パスに入る。 `pending_user_data` が空の場合、 `pop_front()` が `None` を返すため `callback` は呼ばれず、 `i_got` だけが +1 される → 後続の drain で `lock_and_copy_bitstream` が成功 (Ok パス) → `pop_front()` が `None` → `.expect()` でパニックする
2. `pending_user_data.push_back()` と `drain_one_with_ctx` の呼び出し回数にバグがある異常状態 (flush/terminate 時の drain ループで `i_got` が `i_to_send` と等しくなる前に想定外の回数の drain が走るケースなど)

## 問題箇所

```rust
// encode.rs:1483-1485
let user_data = pending_user_data
    .pop_front()
    .expect("pending_user_data must not be empty during drain");
```

## 問題

`pending_user_data` の長さと drain 回数の不変条件が破れた場合、 Ok パスの `.expect()` でワーカースレッドがパニックする。パニックにより `EncoderState::drop` が走らず CUDA リソースがリークする。

デコーダ側の `drain_frames` (decode.rs:777-790) では同じケースを `if let Some(user_data)` で安全に処理し、 `None` の場合はエラーコールバックを呼んで `break` している。

## 修正方針

`.expect()` を外し、 `None` の場合はエラーコールバックで通知する。

修正後の `drain_one_with_ctx` の Ok マッチアーム:

```rust
Ok((data, timestamp, picture_type)) => {
    if let Some(user_data) = pending_user_data.pop_front() {
        callback(Ok(EncodedFrame {
            data,
            timestamp,
            picture_type,
            user_data,
        }));
    } else {
        callback(Err(Error::new_custom(
            "drain_one_with_ctx",
            "missing user data",
        )));
    }
}
```

### 制御フローについて

`drain_one_with_ctx` は 1 回の呼び出しで 1 フレームを drain する構造であり、後続の `unmap_resource` (encode.rs:1501-1507) による CUDA リソース解放が必要なため、デコーダの `break` パターンは採用できない。

また、 `pending_user_data` が空のまま `lock_and_copy_bitstream` が成功し続けると、 flush/terminate 時の drain ループで `i_to_send - i_got` 回だけ `missing user data` エラーが連続でコールバック通知される。連続エラーは意図的な挙動であり許容する（全バッファの CUDA リソース解放を完了させるため）。

なお、 `None` の場合、 `lock_and_copy_bitstream` が成功しているにもかかわらず `data` (Vec<u8>) は callback に渡されずドロップされる。 CUDA リソースは `unmap_resource` で解放済みであり、この動作に問題はない。

### Err パスについて

`lock_and_copy_bitstream` がエラーを返す Err パス (encode.rs:1494-1498) は issue 0018 の修正対象であり、本 issue のスコープ外。

## 変更対象ファイル

- `src/encode.rs` — `drain_one_with_ctx` の Ok マッチアーム (L1483-1485) を修正する
- `CHANGES.md` — `[FIX]` エントリを `## develop` に追加する

## CHANGES.md 追記予定

`## develop` セクションに以下を追記する:

```markdown
- [FIX] `drain_one_with_ctx` の `.expect()` をエラーハンドリングに置き換える
  - Ok パスで `pending_user_data` が空の場合にパニックせずエラーコールバックで通知する
  - @担当者
```

## テスト戦略

`drain_one_with_ctx` は private 関数であり、 `EncoderState` の内部状態と GPU ハードウェアに強く依存する。そのため本修正の検証は以下で行う:

1. **コードレビュー**: Ok パスの `None` 分岐が正しくエラーコールバックを呼ぶこと、後続の `unmap_resource` と `i_got += 1` が適切に実行されることを確認する
2. **既存 GPU テスト**: 以下のコマンドで既存エンコードテストを実行し、正常系に回帰がないことを確認する:
   ```bash
   cargo test --lib -- encode
   ```

## API 互換性

公開 API に変更なし。修正前はパニックしていたケースが `Err(Error::new_custom("drain_one_with_ctx", "missing user data"))` によるエラーコールバック通知に変わる。正常系の動作は変更されない。

## 他の issue との関係

- **issue 0018** (drain エラー握り潰し修正): 0018 の `pending_user_data.clear()` により後続 drain の Ok パスで `pop_front()` が `None` を返す。 0020 未適用の状態で 0018 を適用すると `.expect()` でパニックするため、 0020 → 0018 の順で適用する必要がある
- **issue 0019** (CUDA コンテキスト管理統一): 0019 で `drain_one_with_ctx` → `drain_one` へのリネームが発生する。 0019 が先に適用される場合、エラーメッセージ中の関数名 `"drain_one_with_ctx"` を `"drain_one"` に調整すること

## 解決方法

`src/encode.rs` の `drain_one` 関数の `Ok` マッチアームから `.expect()` を削除し、`pop_front()` が `None` を返した場合はエラーコールバックで通知するようにした。

これにより `pending_user_data` の不変条件が破れた場合のパニックが防止される。
