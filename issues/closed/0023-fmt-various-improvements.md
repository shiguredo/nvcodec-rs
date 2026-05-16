# 0023-fmt-various-improvements

Completed: 2026-05-16
Branch: feature/change-various-improvements
Created: 2026-05-10
Model: deepseek-v4-pro

## 背景

`src/decode.rs` と `src/encode.rs` には、非同期コールバック方式への移行 (issue 0013, 0014) の過程でコード品質低下が蓄積している。#改善項目一覧 に示す 6 項目の改善を行う。全項目とも内部リファクタリングであり、公開 API の振る舞いに変更はない（項目 3 のみレシーバを `&mut self` → `&self` に緩和する `[UPDATE]` 相当の変更を含む）。

## 改善項目一覧

### 1. FFI コールバック 3 関数の重複パターンを共通化

- **種別**: misc
- **対象**: `src/decode.rs:391-585`
- **内容**: `handle_video_sequence`、`handle_picture_decode`、`handle_picture_display` の 3 関数で、NULL チェック → `catch_unwind` → エラー送信のパターンが重複している。
- **注意点**:
  - 3 つの inner 関数の戻り値型が異なる: `handle_video_sequence_inner` は `Result<i32, Error>` を返し、成功時に戻り値 `i32` (= `num_surfaces`) をそのまま返す必要がある。一方 `handle_picture_decode_inner` と `handle_picture_display_inner` は `Result<(), Error>` であり、成功時は固定値 `1` を返す。
  - `handle_picture_display_inner` のみ `&DecoderState`（不変借用）を取る。他 2 つは `&mut DecoderState`。
  - `catch_unwind` の Err ハンドラでのエラーコンテキスト文字列が 3 関数で異なる（`"handle_video_sequence"` / `"handle_picture_decode"` / `"handle_picture_display"`）。
  - 上記の差異を吸収するため、関数ではなく `macro_rules!` で共通化する。マクロは FFI 関数名リテラル、inner 関数呼び出し式、成功時戻り値式、エラーコンテキスト文字列をパラメータとして受け取り、`unsafe extern "C" fn` を生成する。

### 2. デコードテストの検証ロジック重複をヘルパー関数化

- **種別**: misc
- **対象**: `src/decode.rs:892-1345`
- **内容**: 5 つのブラックフレームデコードテストで、Y/UV サイズ検証・ストライド検証・色値平均値検証が重複している。`assert_black_frame(frame: &DecodedFrame<()>, expected_width: usize, expected_height: usize)` ヘルパー関数を定義し、サイズ・ストライド・色値の全検証をこの関数内で実行する。`println!` はヘルパー関数に含めず、各テストの出力は個別に残す。
- **補足**: テスト間で Annex B start code 構築やデコーダー生成定型コードも重複しているが、コーデックごとにテストデータが異なるため本項目のスコープ外とする。

### 3. `Encoder` のメソッド間で `&self` / `&mut self` の一貫性がない

- **種別**: [UPDATE]
- **対象**: `src/encode.rs:1252` (`reconfigure: &mut self`), `src/encode.rs:1264` (`get_sequence_params: &mut self`)
- **内容**: `reconfigure` と `get_sequence_params` は内部で `job_tx.send()` → `rx.recv()` のみを行い、処理の実体はワーカースレッド上の `EncoderState` に委譲されている。`Encoder<T>` 自身の状態を変更しないため `&mut self` は不要。`&self` に変更することで、呼び出し元の不要な `mut` 束縛が解消される。
- **補足**:
  - `EncoderState::get_sequence_params` (L890) も `&mut self` だが、処理の実体である `get_sequence_params_inner` (L895) は `&self` であり、`EncoderState` の可変借用を必要としない。同様に `&self` 化する。
  - `flush` (L1238) は既に `&self` であり、統一後の `Encoder<T>` の全メソッドのレシーバは `&self` で一貫する。
- **テストコードへの影響**:
  - `test_get_sequence_params_h264` (L1584), `test_get_sequence_params_h265` (L1610), `test_get_sequence_params_av1` (L1636): `let mut encoder` → `let encoder` に変更可能。
  - `test_reconfigure_h264` (L1904): `let mut encoder` → `let encoder` に変更可能。
- **後方互換**: `&mut self` は `&self` が必要な箇所で自動的に再借用されるため、呼び出し元の既存コードは変更なしでコンパイル可能。種別は `[UPDATE]`。

### 4. `EncoderState::expected_frame_size` フィールドを削除

- **種別**: misc
- **対象**: `src/encode.rs:457`
- **内容**: フィールドは書き込みのみで一度も読み取られていない。`encode_frame` と `init_buffer_pool` では毎回 `self.buffer_format_enum.frame_size()` を再計算しており、キャッシュとして機能していない。issue 0004/0007/0015 の履歴を確認し、削除が過去の設計意図を損なわないことを確認済み。
- **対応**: フィールド宣言 (L457)、`new()` 内の代入 (L536-538)、`reconfigure_inner` 内の代入 (L756-758) を削除する。

### 5. `EncoderState` の不要な `pub` を削除

- **種別**: misc
- **対象**: `src/encode.rs:566` (`query_caps`), `src/encode.rs:680` (`reconfigure`), `src/encode.rs:890` (`get_sequence_params`)
- **内容**: `EncoderState` はプライベート構造体であり、これらのメソッドは同一モジュール内からのみ呼び出されている。`pub` は不要。

### 6. `DecoderState` の不要な `pub` を削除

- **種別**: misc
- **対象**: `src/decode.rs:100` (`new`), L113 (`query_caps`), L236 (`decode`), L255 (`send_eos`), L275 (`next_frame`)
- **内容**: `DecoderState` はプライベート構造体であり、これら 5 つのメソッドはすべて同一モジュール内からのみ呼び出されている。`pub` は不要。

## 変更対象ファイル

- `src/decode.rs`: 項目 1 (FFI コールバックマクロ化), 項目 2 (テストヘルパー追加), 項目 6 (DecoderState の pub 削除)
- `src/encode.rs`: 項目 3 (&self 統一), 項目 4 (expected_frame_size 削除), 項目 5 (EncoderState の pub 削除)
- `CHANGES.md`: 追記

## テスト戦略

全項目とも内部リファクタリングであり、新たな公開 API やロジックの追加はない。既存テストの回帰確認とコンパイル確認で十分であり、新規テストの追加は不要。

```bash
cargo test --lib -- decode
cargo test --lib -- encode
```

上記コマンドで既存テストを実行し、正常系に回帰がないことを確認する。項目 3 のレシーバ変更後も既存の呼び出し元がコンパイルエラーにならないことを確認する。テストは GPU が必要なため、GPU のない環境ではスキップされる。

## CHANGES.md 追記予定

`## develop` セクション（UPDATE → ADD → CHANGE → FIX の順）:

```markdown
- [UPDATE] `Encoder::reconfigure` と `Encoder::get_sequence_params` のレシーバを `&self` に統一する
  - ワーカースレッドへの委譲のみを行うため `&mut self` は不要
  - @担当者
```

`### misc` サブセクション:

```markdown
- `DecoderState` の FFI コールバックをマクロで共通化し、重複コードを除去する
  - @担当者
- デコードテストのブラックフレーム検証ロジックをヘルパー関数に抽出する
  - @担当者
- `EncoderState::expected_frame_size` の未使用フィールドを削除する
  - @担当者
- `EncoderState` と `DecoderState` の不要な `pub` を削除する
  - @担当者
```

> **注**: `@担当者` は実装時に実際のユーザー名に置換すること。

## 他の issue との関係

- **issue 0019** (CUDA コンテキスト管理統一): `src/encode.rs` の同一領域を変更するため競合の可能性がある。0019 の番号が小さいため、0019 を先に適用し、0019 完了後のコードベースに対して本 issue の項目 3, 4, 5 を適用する。
- **issue 0015/0018/0020/0022**: 変更箇所が異なるため競合しない。


## 解決方法

以下の内部リファクタリングを行った:

1. **FFI コールバックマクロ化**: `handle_video_sequence`、`handle_picture_decode`、`handle_picture_display` の重複パターンを `ffi_callback!` マクロに共通化した。

2. **デコードテストヘルパー追加**: 5 つのブラックフレームデコードテストの重複検証ロジックを `assert_black_frame` ヘルパー関数に抽出した。

3. **`&mut self` → `&self` 統一**: `Encoder<T>::reconfigure`、`Encoder<T>::get_sequence_params`、`EncoderState::get_sequence_params` を `&self` に変更した。

4. **`expected_frame_size` フィールドを削除**: 一度も読み取られないフィールドを削除した。

5/6. **不要な `pub` を削除**: `EncoderState` と `DecoderState` の内部メソッドから `pub` を削除した。
