## 概要

リザルト画面で、GAME OVER(1問も正解できずに終わった)の場合に、専用のBGMへ切り替え、キャラクターアニメーション(ResultSprite)を表示しない。GAME OVER判定はゲーム種別を問わず、`result.total > 0 && result.correct == 0`とする(プレイ前のダミー結果`total == 0`はGAME OVER扱いにしない)。

## 変更内容

- `app.rs`に`fn is_game_over(result: &GameResult) -> bool { result.total > 0 && result.correct == 0 }`を追加する
- `BgmCategory::ResultFailure`を追加する(`dir_prefix`は`"result_failure/"`)
- `enter_result`で、`is_game_over(&result)`なら`BgmCategory::ResultFailure`、そうでなければ既存の`BgmCategory::Result`からBGMを選ぶ
- `assets/audio/bgm/result_failure/Pondus_Mundi.mp3`を配置する(ファイルはユーザー提供待ち)
- `render_result`で、`is_game_over(result)`の間は`sprite.render(...)`を呼ばない(キャラクターを描かない)

## 対象ファイル

- `src/app.rs`
- `src/audio/mod.rs`
- `assets/audio/bgm/result_failure/`(新規ディレクトリ)

## テスト観点

- `is_game_over`: `total==0`(未プレイ)はfalse、`total>0 && correct==0`はtrue、`correct>0`はfalse
- GAME OVERの結果でenter_resultするとBGMが`BgmCategory::ResultFailure`から選ばれること
- 通常の結果(correct>0)ではBGMが従来通り`BgmCategory::Result`から選ばれること
- GAME OVERの結果画面ではキャラクターが描かれないこと(`sprite.render`が呼ばれない)
- 通常の結果画面では従来通りキャラクターが描かれること(既存テストを維持)
