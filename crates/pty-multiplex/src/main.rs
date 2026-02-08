use std::{
    fmt, fs,
    io::{self, BufWriter, Read, Write},
    path::PathBuf,
    process::exit,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::{
    DefaultTerminal,
    layout::Rect,
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, BorderType, Borders, Padding},
};
use std::collections::HashMap;
use tokio::{
    sync::mpsc::{Sender, channel},
    task::spawn_blocking,
};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
use tui_term::vt100;
use tui_term::widget::{Cursor, PseudoTerminal};

#[derive(Debug)]
struct Spawn {
    program: String,
    title: String,
    current_dir: String,
    args: Vec<String>,
    env: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
struct Size {
    cols: u16,
    rows: u16,
}

fn parse_args(args: impl Iterator<Item = String>) -> Vec<Spawn> {
    let mut invocations = Vec::new();
    let mut current: Option<Spawn> = None;

    let mut args = args.skip(1); // Skip the binary name

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--spawn" => {
                if let Some(inv) = current.take() {
                    invocations.push(inv);
                }
                if let Some(program) = args.next() {
                    current = Some(Spawn {
                        program: program.clone(),
                        title: program,
                        current_dir: std::env::current_dir()
                            .unwrap()
                            .to_string_lossy()
                            .to_string(),
                        args: Vec::new(),
                        env: HashMap::new(),
                    });
                } else {
                    panic!("--spawn requires value")
                }
            }
            "--title" => {
                if let Some(inv) = current.as_mut() {
                    if let Some(title) = args.next() {
                        inv.title = title;
                    } else {
                        panic!("--title requires value")
                    }
                } else {
                    panic!("--title requires a preceding --spawn")
                }
            }
            "--current_dir" => {
                if let Some(inv) = current.as_mut() {
                    if let Some(dir) = args.next() {
                        inv.current_dir = dir;
                    } else {
                        panic!("--current_dir requires value")
                    }
                } else {
                    panic!("--current_dir requires a preceding --spawn")
                }
            }
            "--env" => {
                if let Some(inv) = current.as_mut() {
                    if let Some(env_str) = args.next() {
                        let mut parts = env_str.splitn(2, '=');
                        if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
                            inv.env.insert(key.to_string(), val.to_string());
                        } else {
                            panic!("--env requires value")
                        }
                    }
                } else {
                    panic!("--env requires a preceding --spawn")
                }
            }
            "--arg" => {
                if let Some(inv) = current.as_mut() {
                    if let Some(arg_val) = args.next() {
                        inv.args.push(arg_val);
                    } else {
                        panic!("--arg requires value")
                    }
                } else {
                    panic!("--arg requires a preceding --spawn")
                }
            }
            arg => {
                panic!("unexpected arg {}", arg)
            }
        }
    }

    if let Some(inv) = current.take() {
        invocations.push(inv);
    }

    invocations
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    init_panic_hook();
    let args = std::env::args();
    if args.len() < 2 {
        eprintln!("pass --spawn argument and --env --arg after --spawn");
        exit(1);
    }
    let spawns = parse_args(args);
    let mut terminal = ratatui::init();
    let result = run_smux(&mut terminal, spawns).await;
    ratatui::restore();
    result
}

async fn run_smux(terminal: &mut DefaultTerminal, spawns: Vec<Spawn>) -> io::Result<()> {
    let mut size = Size {
        rows: terminal.size()?.height,
        cols: terminal.size()?.width,
    };

    let mut panes: Vec<PtyPane> = Vec::new();
    let mut active_pane: Option<usize> = None;

    let pane_size = calc_pane_size(size, spawns.len());
    for spawn in spawns.into_iter() {
        let mut cmd = CommandBuilder::new(spawn.program);
        cmd.args(spawn.args);
        cmd.cwd(spawn.current_dir);
        for (k, v) in spawn.env {
            cmd.env(k, v);
        }
        open_new_pane(spawn.title, &mut panes, &mut active_pane, &cmd, pane_size)?;
    }

    loop {
        terminal.draw(|f| {
            let area = f.area();

            let pane_width = if panes.is_empty() {
                area.width
            } else {
                area.width / panes.len() as u16
            };

            for (index, pane) in panes.iter().enumerate() {
                let title = Line::from(format!(" {} ", pane.title));
                let block = Block::default()
                    .borders(Borders::all())
                    .border_type(BorderType::Rounded)
                    .padding(Padding::new(1, 1, 1, 1))
                    .title(title)
                    .style(Style::default().bold().dim());
                let mut cursor = Cursor::default();
                let block = if Some(index) == active_pane {
                    block.style(Style::default().bold().fg(Color::LightGreen))
                } else {
                    cursor.hide();
                    block
                };
                let parser = pane.parser.read().unwrap();
                let screen = parser.screen();
                let pseudo_term = PseudoTerminal::new(screen).block(block).cursor(cursor);
                let pane_chunk = Rect {
                    // Adjust the x coordinate for each pane
                    x: area.x + (index as u16 * pane_width),
                    y: area.y,
                    // Use the calculated pane width directly
                    width: pane_width,
                    height: area.height,
                };
                f.render_widget(pseudo_term, pane_chunk);
            }
        })?;

        if event::poll(Duration::from_millis(10))? {
            tracing::info!("terminal Size: {:?}", terminal.size());
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {
                        if let Some(index) = active_pane {
                            if handle_pane_key_event(&mut panes[index], &key).await {
                                continue;
                            }
                        }
                    }
                },
                Event::Resize(cols, rows) => {
                    tracing::info!("resized to: rows: {} cols: {}", rows, cols);
                    size.rows = rows;
                    size.cols = cols;
                    let pane_size = calc_pane_size(size, panes.len());
                    resize_all_panes(&mut panes, pane_size);
                }
                _ => {}
            }
        }

        cleanup_exited_panes(&mut panes, &mut active_pane);

        if panes.is_empty() {
            return Ok(());
        }
    }
}

fn cleanup_exited_panes(panes: &mut Vec<PtyPane>, active_pane: &mut Option<usize>) {
    let mut i = 0;
    while i < panes.len() {
        if !panes[i].is_alive() {
            let _removed_pane = panes.remove(i);
            if let Some(active) = active_pane {
                match (*active).cmp(&i) {
                    std::cmp::Ordering::Greater => {
                        *active = active.saturating_sub(1);
                    }
                    std::cmp::Ordering::Equal => {
                        if panes.is_empty() {
                            *active_pane = None;
                        } else if i >= panes.len() {
                            *active_pane = Some(panes.len() - 1);
                        }
                    }
                    std::cmp::Ordering::Less => {}
                }
            }
        } else {
            i += 1;
        }
    }
}

fn calc_pane_size(mut size: Size, nr_panes: usize) -> Size {
    //   size.cols -= 2;
    size.cols /= nr_panes as u16;
    size
}

fn resize_all_panes(panes: &mut [PtyPane], size: Size) {
    for pane in panes.iter() {
        pane.resize(size);
    }
}

struct PtyPane {
    title: String,
    parser: Arc<RwLock<vt100::Parser>>,
    sender: Sender<Bytes>,
    master_pty: Box<dyn MasterPty>,
    exited: Arc<AtomicBool>,
}

impl PtyPane {
    fn new(title: String, size: Size, cmd: CommandBuilder) -> io::Result<Self> {
        let pty_system = native_pty_system();
        let pty_pair = pty_system
            .openpty(PtySize {
                rows: size.rows - 4,
                cols: size.cols - 4,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let parser = Arc::new(RwLock::new(vt100::Parser::new(
            size.rows - 4,
            size.cols - 4,
            0,
        )));
        let exited = Arc::new(AtomicBool::new(false));

        {
            let exited_clone = exited.clone();
            spawn_blocking(move || {
                let mut child = pty_pair.slave.spawn_command(cmd).unwrap();
                let _ = child.wait();
                exited_clone.store(true, Ordering::Relaxed);
                drop(pty_pair.slave);
            });
        }

        {
            let mut reader = pty_pair.master.try_clone_reader().unwrap();
            let parser = parser.clone();
            tokio::spawn(async move {
                let mut processed_buf = Vec::new();
                let mut buf = [0u8; 8192];

                loop {
                    let size = reader.read(&mut buf).unwrap();
                    if size == 0 {
                        break;
                    }
                    if size > 0 {
                        processed_buf.extend_from_slice(&buf[..size]);
                        let mut parser = parser.write().unwrap();
                        parser.process(&processed_buf);

                        // Clear the processed portion of the buffer
                        processed_buf.clear();
                    }
                }
            });
        }

        let (tx, mut rx) = channel::<Bytes>(32);

        let mut writer = BufWriter::new(pty_pair.master.take_writer().unwrap());
        // writer is moved into the tokio task below
        tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                writer.write_all(&bytes).unwrap();
                writer.flush().unwrap();
            }
        });

        Ok(Self {
            title,
            parser,
            sender: tx,
            master_pty: pty_pair.master,
            exited,
        })
    }

    fn resize(&self, size: Size) {
        self.parser
            .write()
            .unwrap()
            .set_size(size.rows - 4, size.cols - 4);
        self.master_pty
            .resize(PtySize {
                rows: size.rows - 4,
                cols: size.cols - 4,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }

    fn is_alive(&self) -> bool {
        !self.exited.load(Ordering::Relaxed)
    }
}

async fn handle_pane_key_event(pane: &mut PtyPane, key: &KeyEvent) -> bool {
    let input_bytes = match key.code {
        KeyCode::Char(ch) => {
            let mut send = vec![ch as u8];
            let upper = ch.to_ascii_uppercase();
            if key.modifiers == KeyModifiers::CONTROL {
                match upper {
                    'N' => {
                        // Ignore Ctrl+n within a pane
                        return true;
                    }
                    'X' => {
                        // Close the pane
                        return false;
                    }
                    // https://github.com/fyne-io/terminal/blob/master/input.go
                    // https://gist.github.com/ConnerWill/d4b6c776b509add763e17f9f113fd25b
                    '2' | '@' | ' ' => send = vec![0],
                    '3' | '[' => send = vec![27],
                    '4' | '\\' => send = vec![28],
                    '5' | ']' => send = vec![29],
                    '6' | '^' => send = vec![30],
                    '7' | '-' | '_' => send = vec![31],
                    char if ('A'..='_').contains(&char) => {
                        // Since A == 65,
                        // we can safely subtract 64 to get
                        // the corresponding control character
                        let ascii_val = char as u8;
                        let ascii_to_send = ascii_val - 64;
                        send = vec![ascii_to_send];
                    }
                    _ => {}
                }
            }
            send
        }
        #[cfg(unix)]
        KeyCode::Enter => vec![b'\n'],
        #[cfg(windows)]
        KeyCode::Enter => vec![b'\r', b'\n'],
        KeyCode::Backspace => vec![8],
        KeyCode::Left => vec![27, 91, 68],
        KeyCode::Right => vec![27, 91, 67],
        KeyCode::Up => vec![27, 91, 65],
        KeyCode::Down => vec![27, 91, 66],
        KeyCode::Tab => vec![9],
        KeyCode::Home => vec![27, 91, 72],
        KeyCode::End => vec![27, 91, 70],
        KeyCode::PageUp => vec![27, 91, 53, 126],
        KeyCode::PageDown => vec![27, 91, 54, 126],
        KeyCode::BackTab => vec![27, 91, 90],
        KeyCode::Delete => vec![27, 91, 51, 126],
        KeyCode::Insert => vec![27, 91, 50, 126],
        KeyCode::Esc => vec![27],
        _ => return true,
    };

    pane.sender.send(Bytes::from(input_bytes)).await.ok();
    true
}

fn open_new_pane(
    title: String,
    panes: &mut Vec<PtyPane>,
    active_pane: &mut Option<usize>,
    cmd: &CommandBuilder,
    size: Size,
) -> io::Result<()> {
    let new_pane = PtyPane::new(title, size, cmd.clone())?;
    let new_pane_index = panes.len();
    panes.push(new_pane);
    *active_pane = Some(new_pane_index);
    Ok(())
}

fn init_panic_hook() {
    let log_file = Some(PathBuf::from("/tmp/cli2-pty-mux.log"));
    let log_file = match log_file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            Some(fs::File::create(path).unwrap())
        }
        None => None,
    };

    let subscriber = FmtSubscriber::builder()
        // all spans/events with a level higher than TRACE (e.g, debug, info, warn, etc.)
        // will be written to output path.
        .with_max_level(Level::TRACE)
        .with_writer(Mutex::new(log_file.unwrap()))
        .with_thread_ids(true)
        .with_ansi(true)
        .with_line_number(true);

    let subscriber = subscriber.finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // Set the panic hook to log panic information before panicking
    std::panic::set_hook(Box::new(|panic| {
        let original_hook = std::panic::take_hook();
        tracing::error!("panic error: {}", panic);
        ratatui::restore();

        original_hook(panic);
    }));
    tracing::debug!("set panic hook")
}

impl fmt::Debug for PtyPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parser = self.parser.read().unwrap();
        let screen = parser.screen();

        f.debug_struct("PtyPane")
            .field("screen", screen)
            .field("title:", &screen.title())
            .field("icon_name:", &screen.icon_name())
            .finish()
    }
}
