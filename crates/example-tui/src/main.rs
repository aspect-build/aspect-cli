mod action_screen;
mod dummy_backend;
mod main_screen;

use once_cell::sync::Lazy;
use peak_alloc::PeakAlloc;
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, crossterm};
use std::ffi::CStr;
use std::io::Stdout;
use std::sync::RwLock;
use std::thread;
use std::time::Duration;

use crate::action_screen::ActionScreen;
use crate::main_screen::{MainScreen, ScreenState};

#[global_allocator]
static PEAK_ALLOC: PeakAlloc = PeakAlloc;

static SCREEN: Lazy<RwLock<main_screen::MainScreen>> =
    Lazy::new(|| RwLock::new(MainScreen::default()));
static SCREEN2: Lazy<RwLock<action_screen::ActionScreen>> =
    Lazy::new(|| RwLock::new(ActionScreen::default()));

// static TERMINAL: Lazy<RwLock<Option<Terminal<dummy_backend::AnsiBackend<Stdout>>>>> =
//     Lazy::new(|| RwLock::new(None));

static TERMINAL: Lazy<RwLock<Option<Terminal<CrosstermBackend<Stdout>>>>> =
    Lazy::new(|| RwLock::new(None));

/// Allocates a new boxed slice on the Wasm side and returns a raw pointer to it.
#[unsafe(no_mangle)]
pub fn alloc(len: u32) -> *mut u8 {
    Box::leak(vec![0x00_u8; len as usize].into_boxed_slice()).as_mut_ptr()
}

/// Deallocates the boxed slice allocated via [`init`].
///
/// **Safety**
///
/// It is the callers responsibility to assert that `len` is the same as given to [`init`].
#[unsafe(no_mangle)]
pub unsafe fn dealloc(data: *mut u8, len: u32) {
    let len = len as usize;
    let vec = unsafe { Vec::from_raw_parts(data, len, len) };
    drop(vec)
}

#[unsafe(no_mangle)]
pub extern "C" fn set_screen_state(version: *const i8, len: u32) {
    use serde_json::Value;
    let raw_json = unsafe {
        String::from_utf8_unchecked(Vec::from_raw_parts(
            version as *mut u8,
            len as usize,
            len as usize,
        ))
    };
    let mut screen_state: ScreenState = serde_json::from_str(&raw_json).unwrap();
    screen_state.memory = format!("{} MB", PEAK_ALLOC.current_usage_as_mb());
    SCREEN.write().unwrap().set_state(screen_state);
}

#[unsafe(no_mangle)]
pub extern "C" fn render() {
    let screen = SCREEN.read().unwrap();
    let mut terminal = TERMINAL.write().unwrap();
    let terminal = terminal.as_mut().unwrap();
    terminal.draw(|frame| screen.draw(frame)).unwrap();
}

#[unsafe(no_mangle)]
pub extern "C" fn clear() {
    let mut terminal = TERMINAL.write().unwrap();
    let terminal = terminal.as_mut().unwrap();
    terminal.clear().unwrap();
}

#[unsafe(no_mangle)]
pub extern "C" fn sleep(sleep: u32) {
    thread::sleep(Duration::from_millis(sleep as u64));
}

#[unsafe(no_mangle)]
pub extern "C" fn restore_terminal() {
    use crossterm::execute;
    use ratatui::crossterm::event::DisableMouseCapture;
    use ratatui::crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
    let mut terminal = TERMINAL.write().unwrap();
    let terminal = terminal.as_mut().unwrap();

    //disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .unwrap();
    terminal.show_cursor().unwrap();
}

fn main() -> anyhow::Result<()> {
    // let backend = dummy_backend::AnsiBackend::new(
    //     ratatui::layout::Size {
    //         width: 202,
    //         height: 24,
    //     },
    //     std::io::stdout(),
    // );
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let screen = SCREEN.read().unwrap();
        let stdin = std::io::stdin();
        loop {
            let mut buf = String::new();
            let ok = stdin.read_line(&mut buf);
            if ok.is_err() {
                break;
            }
            terminal.clear()?;
            terminal.draw(|frame| screen.draw(frame)).unwrap();
        }
    }

    TERMINAL.write().unwrap().replace(terminal);
    Ok(())
}
