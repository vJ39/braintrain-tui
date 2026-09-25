mod app;
mod audio;
mod canvas;
mod game;
mod image_backend;
mod stats;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::App;
use image_backend::ImageDedupBackend;

const TICK_RATE: Duration = Duration::from_millis(33);

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    // 画像が2枚以上ある画面で、変わっていない画像を毎フレーム再送してチラつくのを防ぐ
    let backend = ImageDedupBackend::new(CrosstermBackend::new(stdout));
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let mut last_tick = Instant::now();

    while !app.should_quit() {
        terminal.draw(|frame| app.render(frame))?;
        // メニューに戻った直後は、画像プロトコルの残留が端末のスクロールバッファに
        // 溜まっていることがあるのでクリアする。Purgeで画面内容も消えるため、
        // ratatui側のバッファもterminal.clear()でリセットし、次のdraw()で全体を描き直す
        if app.take_pending_scrollback_clear() {
            execute!(terminal.backend_mut(), Clear(ClearType::Purge))?;
            terminal.clear()?;
        }

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
        if last_tick.elapsed() >= TICK_RATE {
            app.update(last_tick.elapsed());
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
