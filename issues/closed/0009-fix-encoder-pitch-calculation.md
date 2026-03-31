# 0009 エンコーダーの pitch が常に width になっている

## 優先度

P0

## 概要

`src/encode.rs` の `register_resource()` と `encode_picture()` で `pitch = self.width` を固定しているが、
NVENC の `pitch` はバイト単位の stride である。

- NV12/YV12/IYUV (8bit planar Y): `width` で正しい
- ARGB/ABGR (8bit packed): `width * 4` が必要
- 10bit 系: `width * 2` が必要

NVENC が誤った stride で入力を読むため、壊れた出力になる。

## 対応方針

`BufferFormat` に `bytes_per_row(width)` メソッドを追加し、pitch 計算をフォーマットごとに分岐する。

## 完了内容

`BufferFormat::bytes_per_row(width)` メソッドを追加し、
`register_resource()` と `encode_picture()` の `pitch` を `self.buffer_format_enum.bytes_per_row(self.width)` に修正した。
