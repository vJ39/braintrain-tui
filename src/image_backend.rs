//! 画像プロトコル(sixel/iTerm2/kitty)の再送を抑止するBackendラッパー。
//!
//! ratatui-imageは画像データ(エスケープシーケンス)を画像左上の1セルのsymbolに丸ごと入れる。
//! ratatuiの`Buffer::diff`は「symbolの表示幅ぶん後続セルを無効化する」ため、
//! 数千文字ある画像データの後ろに続くセルは、前フレームと同じでも毎フレーム出力し直される。
//! 1画面に画像が2枚以上あると、2枚目以降(行優先で後ろにある画像)の画像データが
//! 内容が変わっていなくても毎フレーム端末へ再送され、再描画でチラつく。
//!
//! このラッパーは、画像データのセルについて「同じ位置へ前回送ったものと同一」なら出力を省く。
//! 文字セルはそのまま下位のBackendへ渡す(ratatui本来の差分処理を変えない)。

use std::collections::HashMap;
use std::io;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};

pub struct ImageDedupBackend<B: Backend> {
    inner: B,
    /// 位置ごとに、最後に端末へ送った画像データ。文字で上書きされた位置・画面クリア後は消す
    sent_images: HashMap<(u16, u16), String>,
}

/// 画像プロトコルのデータ(エスケープシーケンス)が入ったセルか。
/// ratatuiの通常の文字セルにESCは入らない(色や装飾はBackend側で付ける)。
fn is_image_payload(cell: &Cell) -> bool {
    cell.symbol().starts_with('\x1b')
}

impl<B: Backend> ImageDedupBackend<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            sent_images: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub fn inner(&self) -> &B {
        &self.inner
    }
}

impl<B: Backend> Backend for ImageDedupBackend<B> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let sent_images = &mut self.sent_images;
        let filtered = content.filter(|&(x, y, cell)| {
            if !is_image_payload(cell) {
                // 文字で上書きされたら、そこにあった画像は端末上で壊れているので次回は送り直す
                sent_images.remove(&(x, y));
                return true;
            }
            if sent_images.get(&(x, y)).map(String::as_str) == Some(cell.symbol()) {
                return false;
            }
            sent_images.insert((x, y), cell.symbol().to_string());
            true
        });
        self.inner.draw(filtered)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        // 行が送られると画面上の画像の位置がずれるので、送信済みの記録は捨てる
        self.sent_images.clear();
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        // 画面クリア(リサイズ時もratatuiが呼ぶ)で画像は消えるので、次回は全部送り直す
        self.sent_images.clear();
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        // 部分クリアでもどの画像が消えたか判別しないので、全部送り直す側に倒す
        self.sent_images.clear();
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 終了時の`execute!(terminal.backend_mut(), LeaveAlternateScreen, ...)`のため、
/// 書き込みは下位のBackendへそのまま渡す
impl<B: Backend + io::Write> io::Write for ImageDedupBackend<B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.inner)
    }
}

/// テスト用: drawに渡されたセルを1回のdraw呼び出しごとに記録するBackend
#[cfg(test)]
pub struct RecordingBackend {
    inner: ratatui::backend::TestBackend,
    /// draw呼び出しごとの(x, y, symbol)の列
    pub draws: Vec<Vec<(u16, u16, String)>>,
}

#[cfg(test)]
impl RecordingBackend {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            inner: ratatui::backend::TestBackend::new(width, height),
            draws: Vec::new(),
        }
    }

    /// 直近のdrawで出力された画像データ(ESC始まりのsymbol)のセル位置
    pub fn last_payload_positions(&self) -> Vec<(u16, u16)> {
        self.draws
            .last()
            .map(|cells| {
                cells
                    .iter()
                    .filter(|(_, _, symbol)| symbol.starts_with('\x1b'))
                    .map(|&(x, y, _)| (x, y))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
impl Backend for RecordingBackend {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let cells: Vec<(u16, u16, &Cell)> = content.collect();
        self.draws.push(
            cells
                .iter()
                .map(|&(x, y, cell)| (x, y, cell.symbol().to_string()))
                .collect(),
        );
        self.inner.draw(cells.into_iter())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;
    use ratatui::{Frame, Terminal};

    const LEFT: (u16, u16) = (2, 1);
    const RIGHT: (u16, u16) = (22, 1);

    /// ratatui-imageのsixel描画と同じ形で、1セルに長い画像データを入れ、後続セルをskipにする
    fn put_payload(frame: &mut Frame, pos: (u16, u16), data: &str) {
        let buf = frame.buffer_mut();
        buf[pos].set_symbol(data);
        for x in pos.0 + 1..pos.0 + 8 {
            buf[(x, pos.1)].set_skip(true);
        }
    }

    fn fake_sixel(tag: char) -> String {
        // 実際のsixelと同じく「ESC P ... ESC \」で、表示幅が大きくなる長さにする
        format!("\x1bPq{}\x1b\\", tag.to_string().repeat(500))
    }

    fn two_images(frame: &mut Frame, left: &str, right: &str) {
        frame.render_widget(Paragraph::new("HUD"), Rect::new(0, 0, 10, 1));
        put_payload(frame, LEFT, left);
        put_payload(frame, RIGHT, right);
    }

    #[test]
    fn plain_ratatui_diff_resends_the_later_image_every_frame() {
        // 原因の固定: ラッパー無しだと、左の画像データの表示幅で後続セルが無効化され、
        // 内容が同じでも右の画像データが毎フレーム出力される
        let (l, r) = (fake_sixel('a'), fake_sixel('b'));
        let mut terminal = Terminal::new(RecordingBackend::new(40, 4)).unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        assert_eq!(terminal.backend().last_payload_positions(), vec![RIGHT]);
    }

    #[test]
    fn first_frame_sends_every_image() {
        let (l, r) = (fake_sixel('a'), fake_sixel('b'));
        let mut terminal =
            Terminal::new(ImageDedupBackend::new(RecordingBackend::new(40, 4))).unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        assert_eq!(
            terminal.backend().inner().last_payload_positions(),
            vec![LEFT, RIGHT]
        );
    }

    #[test]
    fn unchanged_images_are_not_resent() {
        let (l, r) = (fake_sixel('a'), fake_sixel('b'));
        let mut terminal =
            Terminal::new(ImageDedupBackend::new(RecordingBackend::new(40, 4))).unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        for _ in 0..3 {
            terminal.draw(|f| two_images(f, &l, &r)).unwrap();
            assert!(terminal
                .backend()
                .inner()
                .last_payload_positions()
                .is_empty());
        }
    }

    #[test]
    fn changed_image_is_sent_again() {
        let (l, r, r2) = (fake_sixel('a'), fake_sixel('b'), fake_sixel('c'));
        let mut terminal =
            Terminal::new(ImageDedupBackend::new(RecordingBackend::new(40, 4))).unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        terminal.draw(|f| two_images(f, &l, &r2)).unwrap();
        assert_eq!(
            terminal.backend().inner().last_payload_positions(),
            vec![RIGHT]
        );
    }

    #[test]
    fn images_are_resent_after_terminal_clear() {
        let (l, r) = (fake_sixel('a'), fake_sixel('b'));
        let mut terminal =
            Terminal::new(ImageDedupBackend::new(RecordingBackend::new(40, 4))).unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        terminal.clear().unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        assert_eq!(
            terminal.backend().inner().last_payload_positions(),
            vec![LEFT, RIGHT]
        );
    }

    #[test]
    fn image_is_resent_after_text_overwrote_its_cell() {
        // 画面遷移等で画像の位置に文字が描かれた後、同じ画像を再び描いたら送り直す
        let (l, r) = (fake_sixel('a'), fake_sixel('b'));
        let mut terminal =
            Terminal::new(ImageDedupBackend::new(RecordingBackend::new(40, 4))).unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        terminal
            .draw(|f| f.render_widget(Paragraph::new("text screen"), f.area()))
            .unwrap();
        terminal.draw(|f| two_images(f, &l, &r)).unwrap();
        assert_eq!(
            terminal.backend().inner().last_payload_positions(),
            vec![LEFT, RIGHT]
        );
    }

    #[test]
    fn text_cells_are_passed_through_unchanged() {
        // 文字セルはratatuiの差分結果をそのまま渡す(画像データのセルだけを間引く)
        let draw_text =
            |f: &mut Frame, s: &str| f.render_widget(Paragraph::new(s.to_string()), f.area());
        let mut plain = Terminal::new(RecordingBackend::new(20, 2)).unwrap();
        let mut wrapped =
            Terminal::new(ImageDedupBackend::new(RecordingBackend::new(20, 2))).unwrap();
        for s in ["hello", "hello", "world"] {
            plain.draw(|f| draw_text(f, s)).unwrap();
            wrapped.draw(|f| draw_text(f, s)).unwrap();
        }
        assert_eq!(plain.backend().draws, wrapped.backend().inner().draws);
    }
}
