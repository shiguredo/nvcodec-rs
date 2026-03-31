# 0010 nvEncLockBitstream の返却ポインタに NULL/0 チェックがない

## 優先度

P1

## 概要

`src/encode.rs` の `lock_and_copy_bitstream()` で `nvEncLockBitstream` 成功後に
`from_raw_parts` を無条件で呼んでいる。

`bitstreamBufferPtr` が NULL または `bitstreamSizeInBytes` が 0 の場合、UB になる。

## 対応方針

- `bitstreamBufferPtr.is_null()` の場合はエラーを返す
- `bitstreamSizeInBytes == 0` の場合は空 `Vec` を返す

## 完了内容

`lock_and_copy_bitstream()` で `from_raw_parts` の前に NULL チェックと 0 byte チェックを追加した。
NULL の場合は `Error::new_custom` でエラーを返し、0 byte の場合は空 `Vec` を返す。
