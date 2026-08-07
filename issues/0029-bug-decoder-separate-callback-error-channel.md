# 0029-bug-decoder-separate-callback-error-channel

- Created: 2026-08-07
- Branch: feature/fix-decoder-callback-error-channel

## 目的

`Decoder` のデコード処理で以下 2 バグを同時に解消する。両者は `state.frame_rx` (`Receiver<Result<RawFrame, Error>>`) に「デコード結果 (Ok)」と「コールバックエラー (Err)」を混在させている構造が根本原因のため、コールバックエラーを別経路に分離することで一度に解消する。

- **バグ A (二重通知)**: `state.decode()` 失敗時にトップレベルのエラーが利用者に通知される一方、シーケンス / picture コールバックが既に `frame_tx.send(Err(...))` で具体的エラーを送っており、次の drain 時に同じ事象が 2 回通知される
- **バグ B (`drain_frames` の scorched-earth)**: `drain_frames` がチャネルから `Err` を取り出すと `pending_user_data.clear()` を実行し、以降の全 user_data ペアリングが破綻する

## 現状

### バグ A の再現経路

`src/decode.rs`:

- `handle_video_sequence` / `handle_picture_decode` / `handle_picture_display` の 3 コールバックラッパーが失敗時に `state.frame_tx.send(Err(e))` する
- parser 側ではコールバック失敗を汎用 CUDA エラーとして返し、`state.decode()` の戻り値 `Err` に伝播する
- `run_worker` は `state.decode()` の戻り値 `Err` に対して `handler.on_decoded(Err(e.into()))` を通知
- 次の成功 decode で `drain_frames` が `frame_rx` から `Err` を取り出し、`handler.on_decoded(Err(callback_error.into()))` として同じ事象を再通知
- 結果として利用側は同一エラーを 2 回受け取る (かつ順序も入れ替わる可能性あり)

### バグ B の再現経路

`drain_frames` (`src/decode.rs`) の `state.next_frame()` の `Err` 分岐でチャネルから `Err` を 1 件取り出すと `pending_user_data` 全体を消す (scorched-earth)。バグ A の再現経路と組み合わさると、コールバックエラー 1 件で以降の全 user_data が失われ、次以降の Ok フレームが「missing user data」エラーになる。

### バグの深刻度

pre-0024 の実装では `handle_video_sequence` が `Err` を返す経路が非常に少なかった (`cuvid_create_decoder` / `cuvid_destroy_decoder` の失敗のみ) ため、両バグは実運用で顕在化しにくかった。0024 で `handle_video_sequence_inner` に追加された新規 `Err` 経路 (`validate_display_area` / max 超過事前検証 / `cuvid_reconfigure_decoder` 失敗等) により、両バグの発火頻度が上がる。0024 の PR では A / B に対する応急処置が試みられたが (`frame_rx` を eager drain する方式)、その修正が別の問題 (Ok フレームも同時に drop する) を生んだため、0024 マージ前に revert される予定。本 issue でクリーンに再修正する。

## 設計方針

### コールバックエラー用の slot を追加する

`DecoderState` に以下のフィールドを追加する:

```rust
callback_error: Mutex<Option<Error>>,
```

3 コールバックラッパー (`handle_video_sequence` / `handle_picture_decode` / `handle_picture_display`) は失敗時に `frame_tx.send(Err(e))` する代わりに `callback_error` slot に格納する。

- コールバックはワーカースレッド上で同期的に呼ばれるため、slot への書き込みは 1 度に 1 スレッド。`Mutex` overhead は無視できる
- 複数コールバックが失敗した場合は「最初の 1 件」を保持する (`take` されるまで上書きしない)。原則は「最初に発生したエラーが根本原因である」ことが多いため最初の 1 件を保持する方が診断しやすい

### `run_worker` の変更

`state.decode()` が `Err` を返した場合:

1. `callback_error` slot を `take()` して取り出す
2. `Some(callback_error)` なら `handler.on_decoded(Err(callback_error.into()))` を通知
3. `None` (コールバック起因ではない driver-level エラー等) なら `handler.on_decoded(Err(e.into()))` を通知
4. 失敗パケットの `user_data` は `pending_user_data` に push しない (現行と同じ)

### `frame_rx` の型変更

`frame_rx` は Ok のみを流す `Receiver<RawFrame>` に変更する。実装方針は 2 択で、実装時に決める:

- **案 A**: `state.frame_rx` / `state.next_frame()` の型を Ok 専用に変更 (`state.next_frame()` は pub API のため破壊的変更に該当)
- **案 B**: pub API シグネチャは `Result<Option<RawFrame>, Error>` のまま維持し、`Err` 分岐が never happen になるだけの実装にする (rustdoc に「本 API は Err を返さなくなった」旨追記)

いずれも `drain_frames` の `Err` 分岐は到達不能になるため削除する。案 A / 案 B の選択は API 破壊性と実装のクリーンさのトレードオフで、実装時に判断する。

### バグ A / B の同時解消

- コールバックエラーが `frame_rx` に流れなくなるため、次の drain で 2 回目通知が発生しない → **バグ A 解消**
- `drain_frames` の `Err` 分岐が到達不能になるため削除でき、scorched-earth も消滅 → **バグ B 解消**

## 完了条件

- `DecoderState` に `callback_error: Mutex<Option<Error>>` フィールドが追加され、3 コールバックラッパーが失敗時に slot へ格納するようになっている (`frame_tx` への `Err` 送信は削除)
- `state.frame_rx` が Ok のみを流す形になり、`drain_frames` の `Err` 分岐が削除されている (実装方針は案 A / 案 B のいずれか)
- `run_worker` の `state.decode()` `Err` 経路で `callback_error` slot を確認し、`Some` なら slot の error を、`None` なら decode の error を 1 回だけ通知する
- 二重通知が発生しないことのテストがある (コールバックエラーを意図的に発生させて、通知回数を検証)
- `drain_frames` の scorched-earth 挙動が消えたことのテストがある (コールバックエラー後も `pending_user_data` が保持され、以降の Ok フレームが正しくペアリングされる検証)
- `CHANGES.md` に `[FIX]` エントリが追加されている (案 A を選んだ場合は `[CHANGE]` エントリも併記)

## 解決方法

### 変更対象ファイル

- `src/decode.rs`:
  - `DecoderState` に `callback_error: Mutex<Option<Error>>` フィールド追加
  - `handle_video_sequence` / `handle_picture_decode` / `handle_picture_display` の `Err` 分岐を `frame_tx.send(Err(e))` から slot への格納に変更
  - `state.frame_rx` の型変更 (案 A / 案 B のいずれか)
  - `state.next_frame()` の戻り型変更 (案 A の場合) or rustdoc 追記 (案 B の場合)
  - `drain_frames` の `Err` 分岐削除
  - `run_worker` の `state.decode()` `Err` 経路を `callback_error` slot 確認ロジックに書き換え
  - テスト追加 (2 バグの解消検証)
- `CHANGES.md` — 追記例:
  - `- [FIX] デコーダーのコールバックエラー通知を frame_rx から分離してバグを解消する`
  - `  - 二重通知バグ (state.decode 失敗時にトップレベルエラーと callback エラーが両方通知される)`
  - `  - drain_frames scorched-earth バグ (callback Err で pending_user_data 全体がクリアされる)`
  - `  - @担当者`

## 関連 issue

- 0024: 本 issue の 2 バグを応急処置しようとした変更を含む。0024 のブランチで一度実装されたが Ok frame drop の副作用があり revert される予定。本 issue でクリーンに再修正
- 0018 (closed): drain エラーが silently drop されていた過去問題。近縁トピックだが本 issue とは別の対応
