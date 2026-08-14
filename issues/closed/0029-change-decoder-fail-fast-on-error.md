# 0029-change-decoder-fail-fast-on-error

- Created: 2026-08-07
- Completed: 2026-08-14
- Branch: feature/change-decoder-fail-fast-on-error
- Polished: 2026-08-13

**本 issue は実装が完了しました。** 一度デコードに失敗した `Decoder` は使えなくなり、復旧は新しい `Decoder` を作る。契約テストは 0024 の `max_coded_*` 超過経路と一緒に後追いする。

## 目的

デコード処理で一度エラーが発生した `Decoder` を終端状態にし、以降のデコードを行わないようにする。

現行の `run_worker` は `DecoderState::decode` が失敗しても `DecodeHandler::on_decoded(Err)` を呼んで `continue` する。これは一過性エラーを想定した設計ではなく、同期 API 時代に `decode()` 失敗後も利用側が呼び続けられた名残である。非同期ワーカーになった今、この継続が次の 2 バグを生んでいる。

- **二重通知**: 失敗した `DecoderState::decode` の戻り値で 1 回通知したあと、コールバックが `frame_tx` に送った同じ事象の `Err` が次の `drain_frames` で再通知される
- **scorched-earth**: `drain_frames` がチャネルから `Err` を取り出すと `pending_user_data.clear()` し、以降の user_data ペアリングが破綻する

継続をやめて終端にすれば、エラー経路の分離装置を足さなくても両バグは消える。復旧は `Decoder` を作り直す。

## 現状

`src/decode.rs` の `run_worker` は `Job::Decode` で `DecoderState::decode` が `Err` のとき、`on_decoded(Err)` のあと `continue` する。このとき:

1. `drain_frames` を呼ばない
2. 失敗パケットの `user_data` を `pending_user_data` に積まない
3. `handle_video_sequence` / `handle_picture_decode` / `handle_picture_display` が失敗時に `frame_tx.send(Err(e))` した具体的エラーと、同じ parse 内で既に積まれた Ok フレームがチャネルに残る

次の成功パケットで `drain_frames` が残り物を取り出し、二重通知と scorched-earth が起きる。NVDEC の `pfnSequenceCallback` 等が 0 を返すと `cuvidParseVideoData` が失敗として伝播する (`nvcuvid.h` の Parser callbacks 節)。一過性のデコードエラーは把握していない。

公開 API (`Decoder::decode` / `DecodeHandler` / README) は、エラー後に同じインスタンスでデコードを続けられるとは書いていない。典型利用は最初の `Err` で止まるか、`Decoder` を drop して作り直す (0025 の想定経路)。

0024 で `handle_video_sequence_inner` に `Err` 経路が増えると、両バグの発火頻度が上がる。0024 の PR では `frame_rx` の eager drain による応急処置が試みられたが、Ok フレームも drop する副作用があり revert 予定。本 issue でエラー後の契約ごと直す。

### 方針変更

当初は `frame_rx` からコールバックエラーを分離し、エラー後もデコードを継続する案だった。継続自体が複雑さの源であり、正しい継続を実装する必要もないため、終端に切り替える。

## 設計方針

### 終端状態

`run_worker` に終端フラグを持たせる。最初のデコードエラーを 1 回通知したあと、以降は `DecoderState::decode` を呼ばない。

- `Job::Decode`: `on_decoded(Err)` する。`decode()` のチャネル送信は成功したままなので、コールバック無しでジョブが消えない
- `Job::Flush`: `send_eos` / `drain_frames` は呼ばない。完了通知だけ返す (`flush()` をハングさせない)
- `Job::Terminate`: 残フレームを drain せず return する。`DecoderState` の Drop で CUDA リソースを解放する

ワーカーをエラー直後に `return` してはいけない。`Decoder::decode` は `SyncSender` (容量 4) への送信成功で `Ok` を返すため、キュー済みジョブのコールバックが消える。

終端後の `on_decoded(Err)` は、最初の失敗とは別のメッセージ (`decoder has already failed` 等) にする。最初の通知だけが原因エラーで、後続は終端であることも利用側から区別できる。

`drain_frames` の missing user data も終端にする。通常あり得ない不変条件違反であり、継続しない。

### 原因エラーの取り出し

`cuvidParseVideoData` はコールバック失敗を汎用 CUDA エラーに潰す。診断のため、`DecoderState` に `callback_error: Option<Error>` を持たせる。

- 3 コールバックラッパーは失敗時に `frame_tx.send(Err(e))` せず、slot が空なら最初の 1 件だけ格納する
- slot 格納後もコールバックは従来どおり失敗 (`0`) を返し続ける。slot 格納でパーサーを成功 (`1`) に切り替えてはいけない。成功を返すと `cuvidParseVideoData` (= `DecoderState::decode`) が Ok になり、`decode` の Err 経路で slot を読み出せなくなるため、slot のエラーが通知されず終端にも入らない
- FFI コールバックは `cuvidParseVideoData` 内でワーカーと同じスレッドから同期的に呼ばれるため `Mutex` は付けない。`handle_picture_display` は現状 `&DecoderState` (共有参照) しか持たないため、slot への書き込みは `Cell<Option<Error>>` を使うか、`handle_picture_display_inner` を `&mut DecoderState` に変更するかで解消する (同期実行で競合しないためどちらでもよい。実装時に選ぶ)
- `DecoderState::decode` が `Err` なら、slot があればそれを、なければ `decode` の戻り値を 1 回だけ通知して終端する

失敗した `decode` と同じ parse 内で既に `frame_tx` に乗った Ok フレームは破棄してよい。デコーダーはこれ以上使えないため、不完全な出力を届けるより終端を優先する。失敗パケットの `user_data` も `pending_user_data` に積まない。

### `frame_rx` は Ok のみ

`frame_tx` / `frame_rx` の型を `Sender<RawFrame>` / `Receiver<RawFrame>` にする。`drain_frames` の `Err` 分岐 (scorched-earth) は削除する。

`DecoderState` は private なので、`next_frame` の戻り型変更は公開 API の破壊的変更ではない。

### rustdoc

`Decoder` / `Decoder::decode` / `DecodeHandler::on_decoded` に、一度 `on_decoded(Err)` が呼ばれたらそのインスタンスは終端であること、復旧は新しい `Decoder` を作ること、を書く。`Decoder::flush` の「flush 後も decode を継続できる」は成功時の話であり、終端後は当てはまらないことを明記する。

エンコーダは本 issue の対象外。buffer full は一過性のため、エラー後もループを続ける現行でよい。

## 利用側への影響

シグネチャは変わらない。公開ドキュメントはエラー後継続を契約していない。現行の「エラー後の継続」は誤った `user_data` や二重 `Err` が付くバグ経路であり、正しい継続を失う話ではない。

見え方が変わる点:

- `on_decoded(Err)` のあと `decode()` を送り続けると、Ok フレームは来ず終端エラーだけが届く
- 同一事象の二重通知と missing user data の連鎖は消える
- `decode()` の戻り値は、ワーカー生存中は今と同じく送信成功で `Ok` (終端後もワーカーは落とさない)

`CHANGES.md` は契約を明示する `[CHANGE]` を主エントリにする。二重通知の解消はその結果として書く。

## 完了条件

- `run_worker` が最初のデコードエラーで終端状態に入り、以降 `DecoderState::decode` を呼ばない
- 最初の原因エラーは `on_decoded(Err)` で 1 回だけ通知される (二重通知が無い)
- 終端後の `Job::Decode` にも `on_decoded(Err)` が届き、`Decoder::decode` の送信は成功する
- 終端後の `flush()` がハングしない
- 3 コールバックラッパーが `frame_tx.send(Err(...))` せず、`callback_error: Option<Error>` に最初の 1 件だけ格納する
- `frame_rx` が Ok のみを流し、`drain_frames` の `Err` 分岐が削除されている
- 上記契約のテストは 0024 実装時に追加する (公開 API で安定誘発できる max 超過経路。本 issue では未着手)
- `Decoder` / `Decoder::decode` / `DecodeHandler::on_decoded` / `Decoder::flush` の rustdoc が終端契約を説明している
- `CHANGES.md` に `[CHANGE]` エントリがある

エラーを起こす手段は実装時に選ぶ。公開 API だけで安定再現できる入力があればそれを使う。0024 マージ後なら `max_coded_width` / `max_coded_height` 超過が使える。モックは使わない。

**終端契約テストの扱い（2026-08-14）**: 公開 API だけで安定したデコードエラー誘発が難しいため、契約テストは 0024 実装時に `max_coded_*` 超過経路と一緒に追加する（0024 側にメモ済み）。本 issue の実装（終端・`callback_error`・rustdoc・CHANGES）はテスト以外で進めてよい。

## 解決方法

`src/decode.rs` の `run_worker` に終端フラグを入れ、最初のデコードエラー以降は `DecoderState::decode` を呼ばないようにした。原因エラーは `callback_error` に 1 件だけ格納し、`DecodeHandler::on_decoded` に 1 回通知する。`frame_tx` / `frame_rx` は Ok 専用にし、`drain_frames` が `Err` で `pending_user_data` を全消しする経路を削除した。

終端時点の未出力 pending はコールバックせず捨てる。失敗した parse 内で既に乗った Ok フレームは `discard_queued_frames` で捨てる。終端後の `Decoder::decode` は送信成功のまま終端を表す `Err` をコールバックする。終端後の `flush` は pending を待たず `Ok` で戻る。`send_eos` の失敗でも終端する。

`Decoder` / `Decoder::decode` / `DecodeHandler` / `Decoder::flush` の rustdoc を終端契約に合わせて更新した。`CHANGES.md` に `[CHANGE]` を追記した。終端契約のテストは 0024 実装時に `max_coded_*` 超過経路と一緒に追加する。

## 関連 issue

- 0024: `handle_video_sequence_inner` の `Err` 経路増加で本 issue の発火頻度が上がる。応急の eager drain は revert 予定
- 0017 (pending): recreate 失敗後に毎ジョブ `callback(Err)` が繰り返される症状は終端で消える。destroy-then-create の復旧不能そのものは 0017 のまま
- 0018 (closed): drain エラーが silently drop されていた過去問題。近縁だが別対応
- 0025 (closed): 利用側の復旧は `Decoder` を drop して作り直す経路
