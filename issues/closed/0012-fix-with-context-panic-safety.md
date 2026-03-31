# 0012 with_context() を panic 安全にする

## 優先度

P1

## 概要

`src/lib.rs` の `with_context()` は CUDA コンテキストを `push` して
クロージャを実行し `pop` するが、クロージャ内で panic すると `pop` が呼ばれない。

CUDA コンテキストスタックが壊れ、以降の CUDA 呼び出しが不可解に失敗する。

## 対応方針

`pop` を必ず実行するガード方式に変更する。

## 完了内容

`with_context()` で `cu_ctx_push_current` の直後に `ReleaseGuard` を設置し、
クロージャが panic しても `cu_ctx_pop_current` が必ず実行されるようにした。
