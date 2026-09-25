use std::cell::RefCell;
use std::io::Cursor;

use rand::Rng;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/audio/"]
struct Assets;

/// assets/audio/bgm/ 配下は用途別にサブフォルダで分ける
/// (menu/ = 起動画面・Playing以外、playing/ = ゲームプレイ中、
///  rhythm/ = リズムゲームの楽曲。譜面と同期させるため曲ごとに選んで再生する、
///  result/ = ゲーム終了後のリザルト画面)
#[derive(RustEmbed)]
#[folder = "assets/audio/bgm/"]
struct BgmAssets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeKind {
    Correct,
    Incorrect,
    Transition,
    /// タイトル画面(Splash)でEnter/クリックした時の決定音
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgmCategory {
    Menu,
    Playing,
    /// ゲーム終了後のリザルト画面
    Result,
    /// リズムゲームの楽曲。再生は譜面と対応する曲をトラック名で直接指定するため
    /// ランダム選曲には使わず、曲データとassetsの対応確認(テスト)で参照する
    #[cfg_attr(not(test), allow(dead_code))]
    Rhythm,
}

impl BgmCategory {
    fn dir_prefix(self) -> &'static str {
        match self {
            BgmCategory::Menu => "menu/",
            BgmCategory::Playing => "playing/",
            BgmCategory::Result => "result/",
            BgmCategory::Rhythm => "rhythm/",
        }
    }
}

/// パス(例: "menu/Calculated_Play.mp3")からディレクトリと拡張子を除いた
/// トラック名("Calculated_Play")を求める
fn track_name_from_path(path: &str) -> String {
    let file_name = path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path);
    file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| file_name.to_string())
}

/// 指定カテゴリのBGMトラック名一覧(名前順)
pub fn bgm_tracks_in(category: BgmCategory) -> Vec<String> {
    let prefix = category.dir_prefix();
    let mut names: Vec<String> = BgmAssets::iter()
        .filter(|p| p.starts_with(prefix))
        .map(|p| track_name_from_path(&p))
        .collect();
    names.sort();
    names
}

/// ジュークボックスに表示する全カテゴリ分のBGMトラック名一覧(名前順、重複なし)
pub fn bgm_track_names() -> Vec<String> {
    let mut names: Vec<String> = BgmAssets::iter().map(|p| track_name_from_path(&p)).collect();
    names.sort();
    names.dedup();
    names
}

/// 指定カテゴリからランダムに1曲選ぶ。そのカテゴリに曲が無ければNone
pub fn random_bgm_track(category: BgmCategory) -> Option<String> {
    let tracks = bgm_tracks_in(category);
    if tracks.is_empty() {
        return None;
    }
    let index = rand::thread_rng().gen_range(0..tracks.len());
    Some(tracks[index].clone())
}

impl SeKind {
    fn asset_path(self) -> &'static str {
        match self {
            SeKind::Correct => "se_correct.wav",
            SeKind::Incorrect => "se_incorrect.wav",
            SeKind::Transition => "se_transition.wav",
            SeKind::Confirm => "se_confirm.wav",
        }
    }
}

/// 実際にrodioで音声デバイスへ再生するプレイヤー。
/// 音声デバイスが無い/取得できない環境では初期化時にNoneとなり、以後は何もしない。
pub struct RodioPlayer {
    handle: Option<(OutputStream, OutputStreamHandle)>,
    /// 現在ループ再生しているBGMのSink。次の曲に切り替える/停止するとき使う
    bgm_sink: RefCell<Option<Sink>>,
}

impl RodioPlayer {
    pub fn new() -> Self {
        let handle = OutputStream::try_default().ok();
        Self {
            handle,
            bgm_sink: RefCell::new(None),
        }
    }

    fn play_se(&self, se: SeKind) {
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Some(file) = Assets::get(se.asset_path()) else {
            return;
        };
        let Ok(sink) = Sink::try_new(stream_handle) else {
            return;
        };
        if let Ok(source) = rodio::Decoder::new(Cursor::new(file.data.into_owned())) {
            sink.append(source);
            sink.detach();
        }
    }

    /// 指定トラックをループ再生する。既に再生中のBGMがあれば止めて切り替える
    fn play_bgm_track(&self, track_name: &str) {
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Some(full_path) = BgmAssets::iter().find(|p| track_name_from_path(p) == track_name)
        else {
            return;
        };
        let Some(file) = BgmAssets::get(&full_path) else {
            return;
        };
        let Ok(sink) = Sink::try_new(stream_handle) else {
            return;
        };
        if let Ok(source) = rodio::Decoder::new(Cursor::new(file.data.into_owned())) {
            if let Some(old) = self.bgm_sink.borrow_mut().take() {
                old.stop();
            }
            sink.append(source.repeat_infinite());
            *self.bgm_sink.borrow_mut() = Some(sink);
        }
    }

    fn stop_bgm(&self) {
        if let Some(sink) = self.bgm_sink.borrow_mut().take() {
            sink.stop();
        }
    }
}

thread_local! {
    // rodio::OutputStreamはSend/Syncでないため、シングルスレッドのTUIループ内で
    // thread_localとして保持する
    static PLAYER: RefCell<RodioPlayer> = RefCell::new(RodioPlayer::new());
}

pub fn play_se(se: SeKind) {
    PLAYER.with(|p| p.borrow().play_se(se));
}

pub fn play_bgm_track(track_name: &str) {
    PLAYER.with(|p| p.borrow().play_bgm_track(track_name));
}

pub fn stop_bgm() {
    PLAYER.with(|p| p.borrow().stop_bgm());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_are_distinct_per_se_kind() {
        let paths = [
            SeKind::Correct.asset_path(),
            SeKind::Incorrect.asset_path(),
            SeKind::Transition.asset_path(),
            SeKind::Confirm.asset_path(),
        ];
        for i in 0..paths.len() {
            for j in (i + 1)..paths.len() {
                assert_ne!(paths[i], paths[j]);
            }
        }
    }

    #[test]
    fn every_se_asset_is_embedded() {
        for se in [
            SeKind::Correct,
            SeKind::Incorrect,
            SeKind::Transition,
            SeKind::Confirm,
        ] {
            assert!(
                Assets::get(se.asset_path()).is_some(),
                "{:?}のasset({})が埋め込まれていること",
                se,
                se.asset_path()
            );
        }
    }

    #[test]
    fn track_name_from_path_strips_directory_and_extension() {
        assert_eq!(
            track_name_from_path("menu/Calculated_Play.mp3"),
            "Calculated_Play"
        );
    }

    #[test]
    fn track_name_from_path_without_extension_is_unchanged() {
        assert_eq!(track_name_from_path("no_extension"), "no_extension");
    }

    #[test]
    fn bgm_track_names_includes_calculated_play() {
        let names = bgm_track_names();
        assert!(
            names.iter().any(|n| n == "Calculated_Play"),
            "bgm/menu/Calculated_Play.mp3が一覧に含まれること: {names:?}"
        );
    }

    #[test]
    fn bgm_track_names_has_no_duplicates() {
        let names = bgm_track_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    #[test]
    fn bgm_tracks_in_menu_includes_calculated_play_only() {
        let names = bgm_tracks_in(BgmCategory::Menu);
        assert!(names.iter().any(|n| n == "Calculated_Play"));
        assert!(!names.iter().any(|n| n == "Method_of_Thought"));
    }

    #[test]
    fn bgm_tracks_in_menu_has_two_tracks() {
        let names = bgm_tracks_in(BgmCategory::Menu);
        assert!(names.iter().any(|n| n == "Calculated_Play"));
        assert!(names.iter().any(|n| n == "Primary_Thruster"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn bgm_tracks_in_playing_includes_method_of_thought_only() {
        let names = bgm_tracks_in(BgmCategory::Playing);
        assert!(names.iter().any(|n| n == "Method_of_Thought"));
        assert!(!names.iter().any(|n| n == "Calculated_Play"));
    }

    #[test]
    fn random_bgm_track_returns_a_track_from_the_category() {
        let tracks = bgm_tracks_in(BgmCategory::Playing);
        let picked = random_bgm_track(BgmCategory::Playing);
        assert!(picked.is_some());
        assert!(tracks.contains(&picked.unwrap()));
    }

    #[test]
    fn bgm_tracks_in_playing_has_two_tracks() {
        let names = bgm_tracks_in(BgmCategory::Playing);
        assert!(names.iter().any(|n| n == "Method_of_Thought"));
        assert!(names.iter().any(|n| n == "The_Quiet_Calculation"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn bgm_tracks_in_result_has_new_personal_best_only() {
        let names = bgm_tracks_in(BgmCategory::Result);
        assert_eq!(names, vec!["New_Personal_Best".to_string()]);
    }

    #[test]
    fn bgm_tracks_in_rhythm_has_both_rhythm_songs_only() {
        let names = bgm_tracks_in(BgmCategory::Rhythm);
        assert_eq!(
            names,
            vec![
                "Redline_Response_Time".to_string(),
                "Top_of_the_Leaderboard".to_string()
            ]
        );
    }

    #[test]
    fn rhythm_songs_are_not_mixed_into_playing_or_menu() {
        for category in [BgmCategory::Menu, BgmCategory::Playing] {
            let names = bgm_tracks_in(category);
            assert!(!names.iter().any(|n| n == "Top_of_the_Leaderboard"));
            assert!(!names.iter().any(|n| n == "Redline_Response_Time"));
        }
    }

    #[test]
    fn random_bgm_track_eventually_picks_both_playing_tracks() {
        // 複数曲から選ばれることを、十分な試行回数で統計的に確認する
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            if let Some(name) = random_bgm_track(BgmCategory::Playing) {
                seen.insert(name);
            }
        }
        assert_eq!(seen.len(), 2, "100回試行して両曲が出現するはず: {seen:?}");
    }
}
