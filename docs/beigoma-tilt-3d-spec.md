## 概要

「べー」の盤面(テキスト表示)は、傾けても矩形グリッドの見た目が変わらず、傾きは数値(HUD)でしか分からない。傾き(pitch/roll)に応じて、盤面表示自体が2Dのまま立体的に傾いて見えるよう変形する。対象はテキスト表示のみ(画像表示は対象外、スコープ外)。

## 変更内容

- `tilt_shift(row: f64, tilt: &Tilt) -> (f64, f64)`関数を追加する。盤の中心の行からの距離(`row`)に比例した、セル単位のずれ(x, y)を返す
  - roll(左右の傾き)は板が左右に傾いて見えるようx方向へ、pitch(前後の傾き)は奥行きが傾いて見えるようy方向へずらす
  - 中心の行(`(BOARD_HEIGHT - 1) / 2.0`)ではずれ0。上下に離れるほどずれが大きくなる(板が回転しているように見える)
  - 係数は`TILT_SHIFT_PER_LEVEL`(セル単位)。傾き量は`Tilt::pitch()`/`roll()`を`TILT_MAX`で正規化(-1.0〜1.0)して使う
- `render_board_text`に`tilt: &Tilt`引数を追加し、各セル・ベーゴマの描画位置(`cell_rect`/`top_rect`の結果)に、その行(ベーゴマは`pos.1`)ぶんの`tilt_shift`をピクセル換算して加える(`offset_rect`ヘルパーを新設)
- `BoardRenderer::render`のシグネチャに`tilt: &Tilt`引数を追加し、テキスト表示(`render_board_text`)にのみ渡す。画像表示(`render_image`)は変更しない
- 呼び出し元(`beigoma.rs`)で`self.board_renderer.render(..., &self.tilt)`に変更する

## 対象ファイル

- `src/game/beigoma/render.rs`
- `src/game/beigoma.rs`

## テスト観点

- 傾き0(`Tilt::default()`)では`tilt_shift`が常に(0.0, 0.0)であること
- roll・pitchを掛けた時、盤の上端と下端で逆方向にずれること(中心を軸に反転する)
- 中心の行ではrollがあってもずれが0であること
- `render_board_text`を傾きありで描画すると、傾き0の描画と比べてセルの表示位置がずれること(実際にBufferを比較する統合テスト)
- 画像表示のパス(`render_image`)は傾きの引数を受け取っても内容を変えないこと
