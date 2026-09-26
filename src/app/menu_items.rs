//! メニュー項目のカタログ(項目名・各項目のインデックス・説明文・アイコン)と、
//! メニュー項目からゲームを作るnew_game。ゲームを1つ追加する時はこのファイルと
//! game_start.rs(開始経路)を触る

use crate::game::beigoma::BeigomaGame;
use crate::game::color_stack::ColorStackGame;
use crate::game::count_mania::CountManiaGame;
use crate::game::look_away::{self, LookAwayGame};
use crate::game::memory::MemoryGame;
use crate::game::mental_calc::MentalCalcGame;
use crate::game::mirror_match::MirrorMatchGame;
use crate::game::pattern_fill::PatternFillGame;
use crate::game::puzzle_connect::PuzzleConnectGame;
use crate::game::quick_draw::QuickDrawGame;
use crate::game::reaction::ReactionGame;
use crate::game::sequence::SequenceGame;
use crate::game::shape_rotate::ShapeRotateGame;
use crate::game::{Difficulty, Game};

pub(super) const MENU_ITEMS: [&str; 16] = [
    "図形回転判定",
    "鏡像判定",
    "イロピッタン",
    "暗算スピード",
    "パターン補完",
    "オイカケ",
    "数列予測",
    "組み合わせパズル",
    "カウントメニア",
    "シタケシ",
    "TTR",
    "ハヤウチ",
    "べー",
    // 仮名称。表示名はゲーム側のDISPLAY_NAMEで一元管理する
    look_away::DISPLAY_NAME,
    "ジュークボックス",
    "履歴",
];

/// イロピッタン(3問→4問→3問で難易度が上がる固定10問)
pub(super) const REACTION_ITEM_INDEX: usize = 2;
/// オイカケ(3問→4問→3問で手数が増える固定10問)
pub(super) const MEMORY_ITEM_INDEX: usize = 5;
/// カウントメニア(マウス専用)
pub(super) const COUNT_MANIA_ITEM_INDEX: usize = 8;
/// シタケシ
pub(super) const COLOR_STACK_ITEM_INDEX: usize = 9;
/// リズムゲーム(TTR)は難易度選択の代わりに曲選択を挟む(難易度は常に上級)
pub(super) const RHYTHM_ITEM_INDEX: usize = 10;
/// ハヤウチ
pub(super) const QUICK_DRAW_ITEM_INDEX: usize = 11;
/// べー
pub(super) const BEIGOMA_ITEM_INDEX: usize = 12;
/// ヤッホー(表示名は仮名称。look_away::DISPLAY_NAMEを参照)
pub(super) const LOOK_AWAY_ITEM_INDEX: usize = 13;
pub(super) const JUKEBOX_ITEM_INDEX: usize = MENU_ITEMS.len() - 2;
pub(super) const HISTORY_ITEM_INDEX: usize = MENU_ITEMS.len() - 1;

/// メニュー各項目の一言説明。配列長をMENU_ITEMS.len()にして、項目の追加漏れをコンパイル時に検出する
pub(super) const MENU_DESCRIPTIONS: [&str; MENU_ITEMS.len()] = [
    "回転させた図形が元と同じかを見分ける",
    "鏡に映した図形かどうかを見分ける",
    "文字の色と意味が一致するかを即答する",
    "4択から計算の答えを素早く選ぶ",
    "3x3の規則から空欄に入る図形を選ぶ",
    "光ったパネルの順番を覚えて再現する",
    "数列の法則を見抜いて次の数を当てる",
    "完成形から2つ目のピースを当てる",
    "数字の円を1から順にクリックする(マウス専用)",
    "色ボタンで各列の一番下のブロックを消して盤面を空にする",
    "矢印キーで曲に合わせてステップする",
    "合図が出たら即座に反応する",
    "軽トラの揺れに耐えてベーゴマをゴールへ運ぶ",
    "指さして「ヤー!!」と叫んだ方の逆を向く",
    "BGMを選んで聴く",
    "ゲームごとの反応時間の推移を見る",
];

/// メニュー各項目のカードに描くアイコン画像(assets/image/からの相対パス)。並び順はMENU_ITEMSと同じ
pub(super) const MENU_ICON_PATHS: [&str; MENU_ITEMS.len()] = [
    "menu_icons/shape_rotate.png",
    "menu_icons/mirror_match.png",
    "menu_icons/reaction.png",
    "menu_icons/mental_calc.png",
    "menu_icons/pattern_fill.png",
    "menu_icons/memory.png",
    "menu_icons/sequence.png",
    "menu_icons/puzzle_connect.png",
    "menu_icons/count_mania.png",
    "menu_icons/color_stack.png",
    "menu_icons/rhythm.png",
    "menu_icons/quick_draw.png",
    "menu_icons/beigoma.png",
    "menu_icons/look_away.png",
    "menu_icons/jukebox.png",
    "menu_icons/history.png",
];

pub(super) fn new_game(item: usize, difficulty: Difficulty) -> Box<dyn Game> {
    match item {
        0 => Box::new(ShapeRotateGame::new(difficulty)),
        1 => Box::new(MirrorMatchGame::new(difficulty)),
        // イロピッタン・記憶は難易度を選ばず、3問→4問→3問で初級→中級→上級相当と進む
        REACTION_ITEM_INDEX => Box::new(ReactionGame::new()),
        3 => Box::new(MentalCalcGame::new(difficulty)),
        4 => Box::new(PatternFillGame::new(difficulty)),
        MEMORY_ITEM_INDEX => Box::new(MemoryGame::new()),
        6 => Box::new(SequenceGame::new(difficulty)),
        7 => Box::new(PuzzleConnectGame::new(difficulty)),
        // カウントメニアは難易度を選ばず、ROUND1=初級・ROUND2=中級・ROUND3=上級と進む
        COUNT_MANIA_ITEM_INDEX => Box::new(CountManiaGame::new()),
        // シタケシは難易度を持たず、ROUND1〜3が固定の内容で進む
        COLOR_STACK_ITEM_INDEX => Box::new(ColorStackGame::new()),
        RHYTHM_ITEM_INDEX => unreachable!("rhythm is started via start_rhythm with a song"),
        // ハヤウチは難易度を持たず、10問固定で進む
        QUICK_DRAW_ITEM_INDEX => Box::new(QuickDrawGame::new()),
        // べーは難易度を持たず、ROUND1・ROUND2が固定の内容で進む
        BEIGOMA_ITEM_INDEX => Box::new(BeigomaGame::new()),
        // ヤッホーは難易度を持たず、10問固定(ライフ制)で進む
        LOOK_AWAY_ITEM_INDEX => Box::new(LookAwayGame::new()),
        _ => unreachable!("history is handled without creating a game"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    /// MENU_ITEMSにゲームを追加してnew_gameのmatchを更新し忘れると、
    /// この範囲でunreachable!に到達してpanicし検知できる
    #[test]
    fn new_game_handles_every_game_menu_item() {
        for item in 0..JUKEBOX_ITEM_INDEX {
            // リズムは曲選択が必要なので start_rhythm で別経路で作る
            if item == RHYTHM_ITEM_INDEX {
                continue;
            }
            let _game = new_game(item, Difficulty::Beginner);
        }
    }

    #[test]
    fn count_mania_comes_right_before_color_stack() {
        assert_eq!(MENU_ITEMS[COUNT_MANIA_ITEM_INDEX], "カウントメニア");
        assert_eq!(COUNT_MANIA_ITEM_INDEX + 1, COLOR_STACK_ITEM_INDEX);
    }

    #[test]
    fn new_game_for_count_mania_item_creates_count_mania() {
        // カウントメニアはROUND1〜5で難易度・動きが変わる固定進行なので、渡した難易度によらず
        // 代表値の難易度で記録する
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let game = new_game(COUNT_MANIA_ITEM_INDEX, difficulty);
            let result = game.result();
            assert_eq!(result.game_id, crate::game::count_mania::GAME_ID);
            assert_eq!(
                result.difficulty,
                crate::game::count_mania::SESSION_DIFFICULTY
            );
        }
    }

    #[test]
    fn color_stack_is_the_last_game_before_rhythm() {
        assert_eq!(MENU_ITEMS[COLOR_STACK_ITEM_INDEX], "シタケシ");
        assert_eq!(COLOR_STACK_ITEM_INDEX + 1, RHYTHM_ITEM_INDEX);
    }

    #[test]
    fn new_game_for_color_stack_item_creates_color_stack() {
        // シタケシは難易度を持たないので、渡した難易度によらず固定の記録になる
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let game = new_game(COLOR_STACK_ITEM_INDEX, difficulty);
            let result = game.result();
            assert_eq!(result.game_id, crate::game::color_stack::GAME_ID);
            assert_eq!(
                result.difficulty,
                crate::game::color_stack::SESSION_DIFFICULTY
            );
        }
    }

    #[test]
    fn memory_and_reaction_item_indices_match_menu() {
        assert_eq!(MENU_ITEMS[MEMORY_ITEM_INDEX], "オイカケ");
        assert_eq!(MENU_ITEMS[REACTION_ITEM_INDEX], "イロピッタン");
    }

    #[test]
    fn new_game_for_memory_and_reaction_records_session_difficulty() {
        // 難易度を選ばないので、渡した難易度によらず代表値の難易度で記録する
        for (item, game_id, session_difficulty) in fixed_progression_games() {
            for difficulty in [
                Difficulty::Beginner,
                Difficulty::Intermediate,
                Difficulty::Advanced,
            ] {
                let result = new_game(item, difficulty).result();
                assert_eq!(result.game_id, game_id);
                assert_eq!(result.difficulty, session_difficulty, "item={item}");
            }
        }
    }

    #[test]
    fn jukebox_and_history_are_the_last_two_menu_items() {
        assert_eq!(JUKEBOX_ITEM_INDEX, MENU_ITEMS.len() - 2);
        assert_eq!(HISTORY_ITEM_INDEX, MENU_ITEMS.len() - 1);
        assert_eq!(MENU_ITEMS[JUKEBOX_ITEM_INDEX], "ジュークボックス");
        assert_eq!(MENU_ITEMS[HISTORY_ITEM_INDEX], "履歴");
    }

    #[test]
    fn menu_icon_paths_match_every_menu_item_in_order() {
        let expected = [
            "shape_rotate.png",
            "mirror_match.png",
            "reaction.png",
            "mental_calc.png",
            "pattern_fill.png",
            "memory.png",
            "sequence.png",
            "puzzle_connect.png",
            "count_mania.png",
            "color_stack.png",
            "rhythm.png",
            "quick_draw.png",
            "beigoma.png",
            "look_away.png",
            "jukebox.png",
            "history.png",
        ];
        assert_eq!(MENU_ICON_PATHS.len(), MENU_ITEMS.len());
        for (path, file) in MENU_ICON_PATHS.iter().zip(expected) {
            assert_eq!(*path, format!("menu_icons/{file}"));
        }
    }

    #[test]
    fn every_menu_icon_is_embedded_and_decodes() {
        for path in MENU_ICON_PATHS {
            if PENDING_MENU_ICONS.contains(&path) {
                continue;
            }
            assert!(
                crate::ui::menu_icons::load_icon_image(path).is_some(),
                "{path}: 埋め込まれていて画像としてデコードできること"
            );
        }
    }

    #[test]
    fn rhythm_item_index_points_at_rhythm_menu_item() {
        assert_eq!(MENU_ITEMS[RHYTHM_ITEM_INDEX], "TTR");
    }

    #[test]
    fn quick_draw_comes_right_after_ttr_and_before_beigoma() {
        assert_eq!(MENU_ITEMS[QUICK_DRAW_ITEM_INDEX], "ハヤウチ");
        assert_eq!(RHYTHM_ITEM_INDEX + 1, QUICK_DRAW_ITEM_INDEX);
        assert_eq!(QUICK_DRAW_ITEM_INDEX + 1, BEIGOMA_ITEM_INDEX);
        // 先頭側の既存インデックスはずれない
        assert_eq!(MENU_ITEMS[COUNT_MANIA_ITEM_INDEX], "カウントメニア");
        assert_eq!(MENU_ITEMS[COLOR_STACK_ITEM_INDEX], "シタケシ");
    }

    #[test]
    fn quick_draw_menu_texts_no_longer_use_old_name() {
        assert!(!MENU_ITEMS.contains(&"反射神経"));
        assert!(!MENU_DESCRIPTIONS[QUICK_DRAW_ITEM_INDEX].contains("反射神経"));
    }

    #[test]
    fn new_game_for_quick_draw_item_creates_quick_draw() {
        // ハヤウチは難易度を選ばないので、渡した難易度によらず代表値の難易度で記録する
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let game = new_game(QUICK_DRAW_ITEM_INDEX, difficulty);
            let result = game.result();
            assert_eq!(result.game_id, crate::game::quick_draw::GAME_ID);
            assert_eq!(
                result.difficulty,
                crate::game::quick_draw::SESSION_DIFFICULTY
            );
        }
    }

    #[test]
    fn beigoma_comes_right_after_quick_draw_and_before_look_away() {
        assert_eq!(MENU_ITEMS[BEIGOMA_ITEM_INDEX], "べー");
        assert_eq!(QUICK_DRAW_ITEM_INDEX + 1, BEIGOMA_ITEM_INDEX);
        assert_eq!(BEIGOMA_ITEM_INDEX + 1, LOOK_AWAY_ITEM_INDEX);
        assert_eq!(
            MENU_ICON_PATHS[BEIGOMA_ITEM_INDEX],
            "menu_icons/beigoma.png"
        );
    }

    #[test]
    fn new_game_for_beigoma_item_creates_beigoma() {
        // べーは難易度を選ばないので、渡した難易度によらず固定の難易度で記録する
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let result = new_game(BEIGOMA_ITEM_INDEX, difficulty).result();
            assert_eq!(result.game_id, crate::game::beigoma::GAME_ID);
            assert_eq!(result.difficulty, crate::game::beigoma::SESSION_DIFFICULTY);
        }
    }

    #[test]
    fn look_away_comes_right_after_beigoma_and_before_jukebox() {
        // 表示名は仮名称なので、文字列を直書きせずゲーム側の定数で確かめる
        assert_eq!(
            MENU_ITEMS[LOOK_AWAY_ITEM_INDEX],
            crate::game::look_away::DISPLAY_NAME
        );
        assert_eq!(BEIGOMA_ITEM_INDEX + 1, LOOK_AWAY_ITEM_INDEX);
        assert_eq!(LOOK_AWAY_ITEM_INDEX + 1, JUKEBOX_ITEM_INDEX);
        assert_eq!(
            MENU_ICON_PATHS[LOOK_AWAY_ITEM_INDEX],
            "menu_icons/look_away.png"
        );
    }

    #[test]
    fn look_away_description_does_not_repeat_the_tentative_name() {
        // 改名時に説明文まで直さずに済むよう、説明文に表示名を入れない
        assert!(
            !MENU_DESCRIPTIONS[LOOK_AWAY_ITEM_INDEX].contains(crate::game::look_away::DISPLAY_NAME)
        );
        assert!(!MENU_DESCRIPTIONS[LOOK_AWAY_ITEM_INDEX].is_empty());
    }

    #[test]
    fn new_game_for_look_away_item_creates_look_away() {
        // 難易度を選ばないので、渡した難易度によらず代表値の難易度で記録する
        for difficulty in [
            Difficulty::Beginner,
            Difficulty::Intermediate,
            Difficulty::Advanced,
        ] {
            let result = new_game(LOOK_AWAY_ITEM_INDEX, difficulty).result();
            assert_eq!(result.game_id, crate::game::look_away::GAME_ID);
            assert_eq!(
                result.difficulty,
                crate::game::look_away::SESSION_DIFFICULTY
            );
        }
    }
}
