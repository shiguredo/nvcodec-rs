# 0035-add-p016-decode-support

- Created: 2026-08-18
- Completed: {YYYY-MM-DD} (例: 2024-07-01)
- Branch: feature/add-p016-decode-support
- Polished: {YYYY-MM-DD} (例: 2024-07-15)

## 目的

10bit ストリームを正しくデコードして出力できるようにする。現状は 8bit NV12 のみ対応で、issue 0034 で 10bit 入力を拒否する (fail-fast) 方針のため、10bit 対応は本 issue で行う。

## 現状

- `src/decode.rs` の `SurfaceFormat` enum は `Nv12` (8bit) のみで、`DecoderState::handle_picture_display` のコピー処理 (`uv_row_bytes` ヘルパー、`cuMemcpy2D` の `WidthInBytes`) は 8bit 前提 (1 画素 1 バイト) で実装されている
- 10bit ストリームの出力は P016 (Semi-Planar YUV 4:2:0 16bit) 等になり、各サンプルが 2 バイトのため、現行の 8bit 前提コピー処理ではバイト幅計算が崩れる
- 過去の issue 0005 (closed) で `SurfaceFormat` enum に `P016` 等が設計されたが、実装は `Nv12` のみで、コピー処理の分岐と `DecodedFrame` の拡張は未対応

## 設計方針

- `SurfaceFormat` に 10bit 出力 (例: `P016`) を追加し、`DecoderState::handle_picture_display` のコピー処理と `DecodedFrame` のプレーンアクセスをフォーマット別に分岐させる
- 10bit ストリーム (`bitDepthMinus8 != 0`) を、拒否せず対応フォーマットでデコードする
- 出力フォーマットに応じた Y / UV のサンプル幅 (1 バイト / 2 バイト) を扱う

## pending にする理由

- 対応には `SurfaceFormat` の拡張、コピー処理のフォーマット分岐、`DecodedFrame` の公開 API 拡張 (サンプル幅) など設計・実装の変更が大きく、優先度が低い
- まず issue 0034 で 10bit 入力を拒否する安全側の対応を済ませ、その後で本 issue に着手する

## 完了条件

- `SurfaceFormat` に 10bit 出力フォーマットが追加されている
- 10bit ストリームを対応フォーマットでデコードし、正しい Y / UV データが返ること
- `DecodedFrame` の公開 API が 10bit サンプル幅に対応していること
- `CHANGES.md` にエントリが追加されていること

## 解決方法

- `src/decode.rs` の `SurfaceFormat` enum、`DecoderState::handle_picture_display` のコピー処理、`DecodedFrame` のプレーンアクセスをフォーマット別に対応させる
