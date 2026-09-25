use std::cell::RefCell;
use std::io::Cursor;

use rodio::{OutputStream, OutputStreamHandle, Sink};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/audio/"]
struct Assets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgmTrack {
    Menu,
    Playing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeKind {
    Correct,
    Incorrect,
    Transition,
}

impl BgmTrack {
    fn asset_path(self) -> &'static str {
        match self {
            BgmTrack::Menu => "bgm_menu.ogg",
            BgmTrack::Playing => "bgm_playing.ogg",
        }
    }
}

impl SeKind {
    fn asset_path(self) -> &'static str {
        match self {
            SeKind::Correct => "se_correct.wav",
            SeKind::Incorrect => "se_incorrect.wav",
            SeKind::Transition => "se_transition.wav",
        }
    }
}

pub trait AudioPlayer {
    fn play_bgm(&self, track: BgmTrack);
    fn play_se(&self, se: SeKind);
}

/// 実際にrodioで音声デバイスへ再生するプレイヤー。
/// 音声デバイスが無い/取得できない環境では初期化時にNoneとなり、以後は何もしない。
pub struct RodioPlayer {
    handle: Option<(OutputStream, OutputStreamHandle)>,
}

impl RodioPlayer {
    pub fn new() -> Self {
        let handle = OutputStream::try_default().ok();
        Self { handle }
    }

    fn play_asset(&self, path: &str) {
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Some(file) = Assets::get(path) else {
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
}

impl AudioPlayer for RodioPlayer {
    fn play_bgm(&self, track: BgmTrack) {
        self.play_asset(track.asset_path());
    }

    fn play_se(&self, se: SeKind) {
        self.play_asset(se.asset_path());
    }
}

thread_local! {
    // rodio::OutputStreamはSend/Syncでないため、シングルスレッドのTUIループ内で
    // thread_localとして保持する
    static PLAYER: RefCell<RodioPlayer> = RefCell::new(RodioPlayer::new());
}

pub fn play_bgm(track: BgmTrack) {
    PLAYER.with(|p| p.borrow().play_bgm(track));
}

pub fn play_se(se: SeKind) {
    PLAYER.with(|p| p.borrow().play_se(se));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_are_distinct_per_bgm_track() {
        assert_ne!(BgmTrack::Menu.asset_path(), BgmTrack::Playing.asset_path());
    }

    #[test]
    fn asset_paths_are_distinct_per_se_kind() {
        let paths = [
            SeKind::Correct.asset_path(),
            SeKind::Incorrect.asset_path(),
            SeKind::Transition.asset_path(),
        ];
        for i in 0..paths.len() {
            for j in (i + 1)..paths.len() {
                assert_ne!(paths[i], paths[j]);
            }
        }
    }
}
