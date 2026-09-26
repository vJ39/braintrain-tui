## 概要

「ハヤウチ」(quick_draw)に、フライングを誘うフェイント演出を追加する。フェイントは10問中の後半(6問目以降)でのみ出す。

## 変更内容

- `Phase::Waiting`に`feint_at: Option<Duration>`(フェイントを始めるまでの残り時間。Noneならこのラウンドはフェイント無し)を追加する
- `Phase::Feint { remaining: Duration, resume_waiting: Duration }`を新設する。`remaining`はフェイント表示の残り時間、`resume_waiting`はフェイントが終わった後に戻る本来の待機の残り時間
- フェイントの判定・タイミング決定はCountdown終了時(Waitingへ入る瞬間)に行う
  - 対象: `tracker.total() >= ROUNDS_PER_SESSION / 2`(6問目以降、0始まりで5以降)のラウンドのみ
  - 確率: `FEINT_RATE`(0.5)
  - 待機時間`remaining`が`FEINT_DURATION * 2`未満(VeryShortパターンの短い待機等)ならフェイントは入れない
  - フェイントの開始タイミングは、待機時間の30%が過ぎた頃から、フェイント表示が終わってもまだ間がある範囲(`remaining * 0.3` 〜 `remaining - FEINT_DURATION`)でランダムに選ぶ
- フェイント表示中(`Phase::Feint`)は、本物の合図と同じ「撃て!」の見出し・画像を、本物より紛らわしい色(`FEINT_BG`、合図の緑よりくすんだ黄緑〜山吹色)の背景で出す
- フェイント中に押すと、`Phase::Countdown`・`Phase::Waiting`と同じくフライング扱いにする
- フェイントの持続時間(`FEINT_DURATION`、300ms)が終わったら、`resume_waiting`の残り待機時間から`Phase::Waiting`に戻る(フェイントは1ラウンドにつき最大1回)

## 対象ファイル

- `src/game/quick_draw.rs`

## テスト観点

- 前半(5問目まで、0始まりで0〜4)のラウンドではフェイントが一切発生しないこと
- 後半のラウンドで、十分な回数プレイすればフェイントが発生する場合があること
- フェイント表示中に押すとフライング扱いになること(反応時間は記録されず、パターンの`fail_latency_ms`が記録される)
- フェイントが終わったら通常の待機に戻り、その後合図が出ること
- フェイント表示の背景色(`FEINT_BG`)が本物の合図の背景色(`SIGNAL_BG`)と異なること
- 待機時間が短い(`VeryShort`パターンなど)ラウンドでは、フェイントの余地が無ければ発生しないこと
