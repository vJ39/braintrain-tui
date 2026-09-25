## 概要

新規ゲーム「べー」。軽トラの荷台に載った女の子キャラが持つベーゴマ盤の上で、高速回転するベーゴマを操作し、盤上の唯一のゴールへ導く。軽トラの走行イベント(段差・信号・障害物回避)のたびに盤にGがかかり、盤上の障害物(凹凸)に触れた瞬間のGの大きさによってベーゴマが弾かれたり吹っ飛んだりする。制限時間60秒。吹っ飛んだら即GAME OVER。

## 操作

- 矢印キー(←↑↓→)で盤の傾きを操作する。傾斜レベルは方向ごとに0〜4の5段階(0=水平、4=最大傾斜)
- 矢印キーを押し続けると傾斜レベルが徐々に上がり、離すと徐々に0へ戻る(即座に4段階切り替わるのではなく、一定速度で変化する)
- 盤の傾きに応じて、ベーゴマは慣性・摩擦のある物理(簡易な2D的な速度・加速度)で転がる。プレイヤーは傾きを細かく調整してベーゴマを誘導し、ゴールに導く

## 軽トラの自動走行とGイベント

軽トラはプレイヤーの操作を介さず自動で走り続ける。以下のイベントが時間経過に伴い(ランダムまたは決められたタイミングで)発生し、そのたびに盤にGがかかる。Gは方向(前後・左右)と大きさを持つ値で、盤の傾き操作とは別に、ベーゴマの速度に外力として加わる。

1. **段差**: 下からの衝撃(垂直方向のG)
2. **信号機**: 黄色/赤色で減速する(制動力=Gの大きさは、信号までの距離とブレーキを踏むタイミングに応じて変わる。ぎりぎりで気づくほどGが大きくなる)。青色になったら発進する(その時も前方向のGが発生する)
3. **障害物回避**: 走行中に前方に障害物が現れ、右左折で避けようとした瞬間に横方向のGが発生する

これらのイベントは軽トラ視点(2視点のうち片方)の映像で見せ場として表示し、プレイヤーはイベントの予兆(段差・信号の色・前方の障害物)を見て、Gが来る前に盤の傾きを調整して備える

## 盤上の障害物(凹凸)と摩擦・吹っ飛び判定

- 盤上には具体的な障害物(凹凸)を複数配置する。障害物が無い平坦な場所では、Gがどれだけ大きくてもベーゴマの位置に特別な影響は無い(通常の転がりのみ)
- ベーゴマが障害物に触れた瞬間、その時点でかかっているGの大きさに応じて3段階の結果になる
  - G が高い閾値を超える → 摩擦でベーゴマが吹っ飛び、即GAME OVER
  - G が低い閾値は超えるが高い閾値には届かない → 吹っ飛びはしないが、ボード上を勢いよく("ピューン"と)弾かれるように移動する(位置・速度が急激に変わる)
  - G が低い閾値以下 → 通常の転がりのまま、特別な影響は無い

## 軽トラの走行physics(Gの求め方)

Gの値は固定値やランダム値ではなく、軽トラの速度・加速度を運動方程式でシミュレートして求める。軽トラは常に「現在速度」を持ち、以下のイベントごとに目標速度・目標横位置と、そこに至るまでの残り距離/残り時間から必要な加速度を逆算し、その加速度の大きさをGとして盤に伝える。

- **巡航**: 通常は一定の巡航速度で走る。加速度0、Gなし
- **信号でのブレーキ**: 信号までの残り距離 `d` と現在速度 `v` から、「その信号の手前で確実に止まるために必要な減速度」を `a = v^2 / (2 * d)` で求める。プレイヤー(というよりゲーム側の仮想ドライバー)が信号の色に気づくタイミングが遅いほど、気づいた時点の `d` が小さくなり、結果として `a`(=制動G)が大きくなる。気づくタイミングは「黄色に変わってからの経過時間」等、決められたロジックで決める(実装時に具体化する)
- **信号での発進**: 青になったら、巡航速度に戻るまで一定の目標加速度で加速する。この加速度の大きさが発進時のGになる
- **障害物回避(操舵)**: 障害物までの残り距離 `d` と現在速度 `v` から、避けるために必要な横方向の移動量(車線変更相当の距離)を、時間 `t = d / v` の間に完了させると仮定し、必要な横加速度を `a_lateral = 2 * lateral_distance / t^2` のような式(等加速度運動の公式)で求める。距離が近い・速度が速いほど`t`が短くなり、横Gが大きくなる
- **段差**: 段差の高さ・軽トラの速度から衝撃の大きさ(垂直方向のG)を決める。単純化した式(速度に比例する等)でよい

これらの式の係数・具体的なロジック(信号の距離・障害物の出現位置等)は実装時に見た目のバランスで決めてよいが、「Gがランダムな数値ではなく、軽トラの速度・距離から運動方程式で導かれている」ことを保証すること。

## 進行・終了条件

- 3.2.1.GO!! の演出(既存の`ui::countdown`を再利用)の後、ベーゴマが盤上に投入され回転を始める
- 制限時間60秒
  - 60秒以内にゴール(盤上の唯一のマス)にベーゴマが到達したらクリア成功
  - 吹っ飛んだら即座に失敗としてGAME OVER
  - 60秒経過してもゴールしなければ時間切れで失敗
- リザルトには成功/失敗と、成功時はクリアタイムを記録する

## UI: 2視点構成

画面を上下(または左右)に分割し、常に2つの視点を同時に表示する
- **視点1(軽トラ視点)**: 軽トラの荷台に乗った女の子キャラと、走行中の背景(道路・信号・段差・障害物)。プレイヤーへの「Gが来る予兆」を伝える映像
- **視点2(盤面視点)**: ベーゴマ盤を真上から見た図。障害物・ゴール・ベーゴマの現在位置を表示する、実際の操作対象

## キャラクター・画像アセット

既存のリザルト画面用キャラクター(`assets/image/result_sprite/frame1.png`等、ピンクツインテールのアイドル風の女の子)を再利用し、「軽トラの荷台でベーゴマ盤を支えながら操作している」構図の新規画像を生成する。以下のプロンプト案を画像生成に使う(スタイルは既存キャラと統一するため、原色系のトゥーン/アニメ塗り、太い輪郭線を指定する)。

### 画像1: 軽トラ視点(通常時)
```
Cute chibi anime girl with pink twin-tails, blue headset microphone, blue-and-pink idol outfit
(same design as reference), standing on the flatbed of a small Japanese kei truck driving down
a road, gripping a wooden spinning-top board with both hands to keep it steady, determined
focused expression, motion lines suggesting the truck is moving, bright cel-shaded anime style,
thick black outlines, vivid flat colors, transparent background, front-three-quarter view
```

### 画像2: 軽トラ視点(Gがかかった瞬間・踏ん張り)
```
Same cute chibi anime girl with pink twin-tails on the flatbed of a kei truck, now leaning hard
to one side and gritting her teeth as the truck brakes suddenly, wooden spinning-top board
tilting in her hands, sweat drop, dynamic action pose, cel-shaded anime style, thick black
outlines, vivid flat colors, transparent background
```

### 画像3: ベーゴマ盤(俯瞰・盤面素材)
```
Top-down view of a round wooden spinning-top (beigoma) battle board, weathered wood texture,
scattered raised bumps and grooves as obstacles, one glowing goal hole marked clearly, simple
flat game-board illustration style, muted natural wood colors, no characters, clean top-down
orthographic angle
```

### 画像4: ベーゴマ本体
```
Small metal spinning top (beigoma) toy, viewed from a slight top-down angle, spinning motion
blur lines around it, shiny metallic surface with colorful painted design, simple flat game
asset illustration, transparent background
```

これらは案であり、実際に生成した画像を見てから微調整してよい。

## 対象ファイル(新規)

- 新規モジュール: `src/game/beigoma.rs`
- `src/app.rs`(メニュー項目・new_game・難易度選択スキップの配線。難易度は選ばないゲームとして扱う)
- 新規アセット: `assets/image/beigoma/`配下(上記4画像)

## テスト観点

- 信号ブレーキの制動Gが `a = v^2 / (2 * d)` で求まり、気づくタイミングが遅い(dが小さい)ほどGが大きくなること
- 発進時の加速度、障害物回避時の横Gが、それぞれの式通りに速度・距離から求まること
- 段差の衝撃Gが速度に応じて変わること
- 傾斜レベルが矢印キーの押下で0→4まで段階的に上がり、離すと0へ戻ること
- 盤の傾きに応じてベーゴマの速度・位置が変化すること(物理挙動の基本テスト)
- 段差・信号・障害物回避の各イベントで、想定したGが発生すること
- 障害物にベーゴマが触れた瞬間のGの大きさに応じて、無影響/弾かれる/吹っ飛ぶの3パターンが正しく判定されること
- 吹っ飛んだ場合、即座にGAME OVER(失敗)になること
- 60秒以内にゴールに到達したら成功、到達しなければ時間切れで失敗になること
- 3.2.1.GO!!のカウントダウン後にベーゴマが投入されること
- メニューから選ぶと難易度選択を経由せず直接プレイが始まること
