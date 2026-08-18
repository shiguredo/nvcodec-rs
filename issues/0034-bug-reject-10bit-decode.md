# 0034-bug-reject-10bit-decode

- Created: 2026-08-18
- Completed: {YYYY-MM-DD} (例: 2024-07-01)
- Branch: feature/fix-reject-10bit-decode
- Polished: {YYYY-MM-DD} (例: 2024-07-15)

## 目的

デコーダー出力が 8bit NV12 のみ対応なのに 10bit ストリーム (`bitDepthMinus8 != 0`) を拒否していないため、10bit 入力で不正な出力になり得る。8bit しか対応しない実装なので、10bit 入力を明示的なエラーで拒否する。

## 現状

- `src/decode.rs` の `SurfaceFormat` enum は `Nv12` (8bit) のみで、`DecoderState::handle_picture_display` の NV12 コピー処理 (`uv_row_bytes` ヘルパー、`cuMemcpy2D` の `WidthInBytes`) は 8bit 前提 (1 画素 1 バイト) で実装されている
- 一方、`DecoderState::handle_video_sequence` は `format.bit_depth_luma_minus8` を検証せず `CUVIDDECODECREATEINFO.bitDepthMinus8` へそのまま渡している
- 10bit ストリーム (`bitDepthMinus8 = 2`) が来ると出力サーフェスは P010 相当 (各サンプル 2 バイト) になる一方、コピー処理は 8bit 前提のため Y / UV のバイト幅計算が崩れ、不正な画素データが返る
- issue 0031 の修正 (`uv_row_bytes` による UV 行バイト幅の統合) で 8bit NV12 前提がより明確になったものの、10bit の拒否は未実装のまま残っている

## 設計方針

- 現時点で対応する出力フォーマットは 8bit NV12 のみなので、10bit 入力を `DecoderState::handle_video_sequence` で検証し、`bit_depth_luma_minus8 != 0` の場合に明示的なエラーとして Decoder を終端させる
- 10bit 対応 (P016 等) は出力フォーマットの拡張を伴うため、本 issue では扱わない (別 issue とする)
- 既存の奇数原点の拒否と同様、`handle_video_sequence` の検証で fail-fast にする

## 完了条件

- `bit_depth_luma_minus8 != 0` (10bit 等) のストリームで、`DecoderState::handle_video_sequence` がエラーを返し Decoder が終端すること
- 8bit ストリームの既存挙動に回帰がないこと
- `CHANGES.md` にエントリが追加されていること

## 解決方法

- `src/decode.rs` の `handle_video_sequence` の検証に、`format.bit_depth_luma_minus8 != 0` のチェックを追加する
