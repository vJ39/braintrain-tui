## 概要

イロピッタン(reaction.rs)のセッション問題数を10問から20問に増やす。初級/中級/上級の配分は6問/7問/7問(現行の3:4:3比率に近い形で20問に按分)。

## 変更内容

- `SESSION_LENGTH`を`10`から`20`に変更する
- `question_difficulty(question_index)`の区間を以下に変更する
  - 0〜5問目(1〜6問目): `Difficulty::Beginner`
  - 6〜12問目(7〜13問目): `Difficulty::Intermediate`
  - 13〜19問目(14〜20問目): `Difficulty::Advanced`
- HUDは`ScoreTracker::with_session_length(SESSION_LENGTH)`経由で自動的に`Q_/20`表示になる(コード変更不要)

## 対象ファイル

- `src/game/reaction.rs`

## テスト観点

- `EXPECTED_DIFFICULTIES`を20要素(6+7+7)に拡張し、境界(5問目→6問目、12問目→13問目)で正しく難易度が切り替わること
- セッションが20問で終了すること。HUDが`Q_/20`を表示すること
- 上級相当区間の先頭(14問目、0始まりindex13)から時間制限・漢字/カタカナ混在が有効なこと
- 既存の色プール・表記・正誤表示・画像プロトコル関連のテストが壊れていないこと
