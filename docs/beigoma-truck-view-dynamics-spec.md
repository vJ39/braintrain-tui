## 概要

「べー」の軽トラ視点(左側パネル)に、速度感・前方イベントの接近・Gによる揺れを可視化する。対象はテキスト・画像共通の描画位置(揺れ)と、テキストの強調表示(接近)。軽トラ・キャラクターの構図自体の画像化は別途(#128のプロンプトで生成された画像が届いてから)。

## 変更内容

- `TruckViewInfo`に`elapsed: Duration`(ゲーム開始からの経過時間)を追加する
- `TruckViewRenderer::render`で、女の子の絵(`picture`)の描画位置を、Gの大きさに応じた振幅・`elapsed`ベースの周期でオフセットさせる(`shake_offset`関数)。Gが大きいほど揺れが大きく、小さければ(#135の巡航中の微振動程度なら)ごく僅かに揺れる
  - 揺れのオフセットは`picture`の範囲内でクランプし、はみ出さないようにする
- 前方イベントの案内文(`upcoming_text`)が近い距離(`URGENT_DISTANCE`、15m以内)の時は、強調表示(太字+目立つ色)にする(`upcoming_is_urgent`関数)

## 対象ファイル

- `src/game/beigoma/render.rs`
- `src/game/beigoma.rs`(`TruckViewInfo`生成箇所に`elapsed`を渡す)

## テスト観点

- Gが大きいほど揺れの振幅が大きいこと
- `elapsed`が変わると揺れの向き・大きさが変わること(固定値でないこと)
- 揺れのオフセットが`picture`の範囲を超えてpanicしないこと
- `URGENT_DISTANCE`より近い前方イベントは強調スタイル(BOLD)になり、遠い/無い場合はならないこと
