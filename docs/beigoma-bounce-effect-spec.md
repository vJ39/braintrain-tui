## 概要

「べー」で弾かれた(Landing::Bounce)時の演出を強化する。ベーゴマの回転をもっと激しく見せ、弾かれた時の一言を「ぴよーん!!」という弾かれるアニメーション的な文言と「ああっ!!」というセリフに変える。

## 変更内容

- `SPIN_FRAME_INTERVAL`を`120ms`から`70ms`に短縮し、回転のコマ送りを速くする(見た目の回転を激しくする)
- `Landing::Bounce`(`StepEvent::Landed(Landing::Bounce)`・`StepEvent::Grazed(Landing::Bounce)`)発生時の一言を、`"ピューン!"`から`"ぴよーん!! ああっ!!"`に変更する

## 対象ファイル

- `src/game/beigoma.rs`

## テスト観点

- `SPIN_FRAME_INTERVAL`が`70ms`であること
- `Landing::Bounce`発生時のメッセージが`"ぴよーん!! ああっ!!"`であること(凸で弾かれた時・凹の側面をこすって弾かれた時の両方)
