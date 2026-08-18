# testdata/odd-width のテストデータ

奇数表示幅 (width が奇数) のストリームで、NV12 の UV 行バイト幅が
`ceil(width/2)*2` として正しくコピーされることを検証するためのテストデータ。

## データの性質

| ファイル | コーデック | 寸法 | 内容 |
|---|---|---|---|
| `red_65x65.jpg` | JPEG | 65x65 (奇数) | 単色 (赤) |

- 幅 65 は奇数。NV12 の UV 行バイト幅は `ceil(65/2)*2 = 66` バイトになり、
  33 個のクロマサンプル (U/V インターリーブ) が並ぶ
- 単色 (赤) のため各 UV 行のクロマは一様で、最後のクロマの V (オフセット 65 == width)
  が 0 埋めでないことを判定しやすい
- 修正前は `WidthInBytes = width (=65)` でコピーするためオフセット 65 が 0 のまま
  残っていたが、修正後は 66 バイトコピーされる

## テストでの使われ方

- `src/decode.rs` の `mod tests` が `include_bytes!` で読み込む
- 奇数幅の UV 行バイト幅の回帰テスト (`test_decode_jpeg_odd_width_uv_row_bytes`) に使う
- デコードの実機検証は NVIDIA GPU を要するため、CI の `test-nvidia-video-codec` で実行する
