# 0032-refactor-decoder-callback-methods

- Created: 2026-08-17
- Completed: {YYYY-MM-DD} (例: 2024-07-01)
- Branch: feature/refactor-decoder-callback-methods
- Polished: {YYYY-MM-DD} (例: 2024-07-15)
- Reporter: @sile

## 目的

`src/decode.rs` の decoder コールバック処理を、`DecoderState` を引数に取るフリー関数から `DecoderState` のメソッドへ移す。

このメソッド化は、`feature/change-decoder-reconfigure-with-max-coded-size` ブランチの issue 0024 で予定している変更のうち、reconfigure 実装に依存しない部分だけを先に develop へ取り込むためのリファクタリングである。

## 現状

`src/decode.rs` には、extern "C" ラッパーから呼ばれる次のフリー関数がある。

- `handle_video_sequence_inner(state, format)` — sequence callback の本体
- `handle_picture_decode_inner(state, pic_params)` — decode callback の本体
- `handle_picture_display_inner(state, disp_info)` — display callback の本体

これらはすべて `DecoderState` を第一引数に取り、`DecoderState` のフィールドへ読み書きする。

issue 0024 の reconfigure 経路や、将来の surface 数決定処理では、同じ処理を `DecoderState` のメソッドとして呼び出したい。しかし develop ではフリー関数のままだと、callback 処理を統一的に扱いにくい。

## 設計方針

挙動を一切変えない純粋なリファクタリングとして、次の 3 つのフリー関数を `DecoderState` のメソッドへ移す。

- `handle_video_sequence_inner` → `DecoderState::handle_video_sequence(&mut self, format)`
- `handle_picture_decode_inner` → `DecoderState::handle_picture_decode(&mut self, pic_params)`
- `handle_picture_display_inner` → `DecoderState::handle_picture_display(&self, disp_info)`

メソッドの本体は、現在のフリー関数の実装をそのまま移す。decoder の破棄と再作成という develop の現状経路を維持し、reconfigure 固有の処理（`max_coded_width` / `max_coded_height` の保持と検証、`cuvidReconfigureDecoder` の呼び出し、`ReconfigureBaseline` による変化の判定、`create_geometry` の保存と利用）は含めない。

extern "C" ラッパーは `state.method(...)` の形でメソッドを呼ぶように変更する。

## 完了条件

- `handle_video_sequence_inner` 相当の処理が `DecoderState::handle_video_sequence` メソッドになっている
- `handle_picture_decode_inner` 相当の処理が `DecoderState::handle_picture_decode` メソッドになっている
- `handle_picture_display_inner` 相当の処理が `DecoderState::handle_picture_display` メソッドになっている
- extern "C" ラッパーがメソッドを呼ぶ形に変更されている
- reconfigure 固有の処理が本 issue に含まれていない
- 既存の decoder 初期化 / デコードテストにリグレッションがない

## 解決方法

`src/decode.rs` のみを変更する。

フリー関数 `handle_video_sequence_inner` / `handle_picture_decode_inner` / `handle_picture_display_inner` を削除し、`DecoderState` の `impl` ブロックへ同名メソッドとして追加する。extern "C" ラッパー `handle_video_sequence` / `handle_picture_decode` / `handle_picture_display` の本体を、メソッド呼び出しへ変更する。

## 関連 issue

- 0024: reconfigure と decoder 再作成のハイブリッド化。本 issue のメソッド化を前提に reconfigure 経路を実装する
- 0028: parser と decoder の surface 数を同期する。本 issue のメソッド化を前提に surface 数決定 helper を組み込む
