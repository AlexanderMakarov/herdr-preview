use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseRow {
    Parent { path: PathBuf },
    Dir { name: String, path: PathBuf },
    File { name: String, path: PathBuf },
    EmptyPlaceholder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseState {
    pub cwd: PathBuf,
    pub rows: Vec<BrowseRow>,
    pub selected: usize,
    pub scroll: usize,
    pub notice: Option<String>,
    /// Origin pane cwd; when the listing is outside this tree, the footer
    /// warns that file-viewer will open in a new tab.
    pub origin_cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseCommand {
    MoveUp,
    MoveDown,
    GoParent,
    EnterDir,
    Activate,
    ScrollUp,
    ScrollDown,
    SelectIndex(usize),
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseOutcome {
    Continue,
    OpenFile { path: PathBuf },
    Dismiss,
}

impl BrowseRow {
    pub fn display_name(&self) -> String {
        match self {
            BrowseRow::Parent { .. } => "..".to_string(),
            BrowseRow::Dir { name, .. } => format!("{name}/"),
            BrowseRow::File { name, .. } => name.clone(),
            BrowseRow::EmptyPlaceholder => "(empty)".to_string(),
        }
    }

    pub fn is_activatable(&self) -> bool {
        !matches!(self, BrowseRow::EmptyPlaceholder)
    }
}

impl BrowseState {
    pub fn open(path: &Path) -> Self {
        let cwd = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        match list_rows(&cwd) {
            Ok(rows) => Self {
                cwd,
                rows,
                selected: 0,
                scroll: 0,
                notice: None,
                origin_cwd: None,
            },
            Err(_) => Self {
                cwd: cwd.clone(),
                rows: fallback_rows(&cwd),
                selected: 0,
                scroll: 0,
                notice: Some("cannot read directory".into()),
                origin_cwd: None,
            },
        }
    }

    pub fn apply(&mut self, cmd: BrowseCommand, visible_rows: usize) -> BrowseOutcome {
        let visible_rows = visible_rows.max(1);
        match cmd {
            BrowseCommand::MoveUp => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                self.ensure_visible(visible_rows);
                BrowseOutcome::Continue
            }
            BrowseCommand::MoveDown => {
                if !self.rows.is_empty() && self.selected + 1 < self.rows.len() {
                    self.selected += 1;
                }
                self.ensure_visible(visible_rows);
                BrowseOutcome::Continue
            }
            BrowseCommand::GoParent => {
                if let Some(parent) = self.cwd.parent().map(Path::to_path_buf) {
                    self.try_enter(&parent);
                }
                BrowseOutcome::Continue
            }
            BrowseCommand::EnterDir => match self.rows.get(self.selected) {
                Some(BrowseRow::Dir { path, .. }) => {
                    let path = path.clone();
                    self.try_enter(&path);
                    BrowseOutcome::Continue
                }
                _ => BrowseOutcome::Continue,
            },
            BrowseCommand::Activate => match self.rows.get(self.selected) {
                Some(BrowseRow::Parent { path }) => {
                    let path = path.clone();
                    self.try_enter(&path);
                    BrowseOutcome::Continue
                }
                Some(BrowseRow::Dir { path, .. }) => {
                    let path = path.clone();
                    self.try_enter(&path);
                    BrowseOutcome::Continue
                }
                Some(BrowseRow::File { path, .. }) => {
                    BrowseOutcome::OpenFile { path: path.clone() }
                }
                Some(BrowseRow::EmptyPlaceholder) | None => BrowseOutcome::Continue,
            },
            BrowseCommand::ScrollUp => {
                self.scroll = self.scroll.saturating_sub(1);
                BrowseOutcome::Continue
            }
            BrowseCommand::ScrollDown => {
                let max_scroll = self.max_scroll(visible_rows);
                if self.scroll < max_scroll {
                    self.scroll += 1;
                }
                BrowseOutcome::Continue
            }
            BrowseCommand::SelectIndex(index) => {
                if self.rows.is_empty() {
                    self.selected = 0;
                } else {
                    self.selected = index.min(self.rows.len() - 1);
                }
                BrowseOutcome::Continue
            }
            BrowseCommand::Dismiss => BrowseOutcome::Dismiss,
        }
    }

    fn ensure_visible(&mut self, visible_rows: usize) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
        if self.selected >= self.scroll + visible_rows {
            self.scroll = self.selected + 1 - visible_rows;
        }
    }

    fn max_scroll(&self, visible_rows: usize) -> usize {
        self.rows.len().saturating_sub(visible_rows)
    }

    fn try_enter(&mut self, path: &Path) {
        match list_rows(path) {
            Ok(rows) => {
                self.cwd = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                self.rows = rows;
                self.selected = 0;
                self.scroll = 0;
                self.notice = None;
            }
            Err(_) => self.notice = Some("cannot read directory".into()),
        }
    }
}

fn fallback_rows(path: &Path) -> Vec<BrowseRow> {
    let mut rows = Vec::new();
    if let Some(parent) = path.parent() {
        rows.push(BrowseRow::Parent {
            path: parent.to_path_buf(),
        });
    }
    rows.push(BrowseRow::EmptyPlaceholder);
    rows
}

fn list_rows(path: &Path) -> io::Result<Vec<BrowseRow>> {
    let mut rows = Vec::new();
    if let Some(parent) = path.parent() {
        rows.push(BrowseRow::Parent {
            path: parent.to_path_buf(),
        });
    }

    let mut children = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_path = entry.path();
        // Follow symlinks so symlink-to-dir / symlink-to-file list as Dir / File.
        let Ok(meta) = fs::metadata(&child_path) else {
            continue;
        };
        if meta.is_dir() {
            children.push((true, name, child_path));
        } else if meta.is_file() {
            children.push((false, name, child_path));
        }
    }

    children.sort_by(
        |(a_is_dir, a_name, _), (b_is_dir, b_name, _)| match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a_name.to_lowercase().cmp(&b_name.to_lowercase()),
        },
    );

    let empty = children.is_empty();
    for (is_dir, name, child_path) in children {
        if is_dir {
            rows.push(BrowseRow::Dir {
                name,
                path: child_path,
            });
        } else {
            rows.push(BrowseRow::File {
                name,
                path: child_path,
            });
        }
    }

    if empty {
        rows.push(BrowseRow::EmptyPlaceholder);
    }

    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseKey {
    Char(char),
    Enter,
    Esc,
    Up,
    Down,
    Left,
    Right,
    MouseClick { row: u16, col: u16 }, // 0-based screen row/col
    MouseWheelUp,
    MouseWheelDown,
}

const ESC: u8 = 0x1b;
const ANSI_HOME: &str = "\x1b[H";
const ANSI_CLEAR: &str = "\x1b[J";
const ANSI_HIDE: &str = "\x1b[?25l";
const ANSI_SHOW: &str = "\x1b[?25h";
const ANSI_WRAP_OFF: &str = "\x1b[?7l";
const ANSI_WRAP_ON: &str = "\x1b[?7h";
const ANSI_CLR_EOL: &str = "\x1b[K";
const ANSI_MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
const ANSI_MOUSE_OFF: &str = "\x1b[?1000l\x1b[?1006l";
const DIM: &str = "\x1b[2;90m";
const RESET: &str = "\x1b[0m";
const REV: &str = "\x1b[7m";
const LEGEND: &str = "browse · j/k move · enter open · h parent · l enter · q cancel";
const OUTSIDE_TAB: &str =
    "outside this tab's cwd · file-viewer opens in a new tab · j/k enter q";

pub fn render_browse(state: &BrowseState, rows: u16, cols: u16) -> String {
    let rows = rows.max(2) as usize;
    let cols = cols.max(1) as usize;
    let list_height = rows.saturating_sub(2);

    let cwd = truncate_chars(&state.cwd.display().to_string(), cols);
    let mut lines = Vec::with_capacity(rows);
    lines.push(format!("{DIM}{cwd}{RESET}"));

    let start = state.scroll;
    for i in 0..list_height {
        let index = start + i;
        let line = match state.rows.get(index) {
            Some(row) => {
                let marker = if index == state.selected { '>' } else { ' ' };
                let text = truncate_chars(&format!("{marker} {}", row.display_name()), cols);
                if index == state.selected {
                    format!("{REV}{text}{RESET}")
                } else {
                    text
                }
            }
            None => String::new(),
        };
        lines.push(line);
    }

    let footer = if let Some(notice) = &state.notice {
        notice.as_str()
    } else if state
        .origin_cwd
        .as_ref()
        .is_some_and(|origin| !crate::open::is_under_origin_tree(&state.cwd, origin))
    {
        OUTSIDE_TAB
    } else {
        LEGEND
    };
    lines.push(truncate_chars(footer, cols));
    lines.join("\n")
}

pub fn map_browse_key(key: BrowseKey) -> Option<BrowseCommand> {
    match key {
        BrowseKey::Char('k') | BrowseKey::Up => Some(BrowseCommand::MoveUp),
        BrowseKey::Char('j') | BrowseKey::Down => Some(BrowseCommand::MoveDown),
        BrowseKey::Char('h') | BrowseKey::Left => Some(BrowseCommand::GoParent),
        BrowseKey::Char('l') | BrowseKey::Right => Some(BrowseCommand::EnterDir),
        BrowseKey::Enter => Some(BrowseCommand::Activate),
        BrowseKey::Char('q') | BrowseKey::Char('Q') | BrowseKey::Esc => {
            Some(BrowseCommand::Dismiss)
        }
        BrowseKey::MouseWheelUp => Some(BrowseCommand::ScrollUp),
        BrowseKey::MouseWheelDown => Some(BrowseCommand::ScrollDown),
        BrowseKey::MouseClick { .. } | BrowseKey::Char(_) => None,
    }
}

pub fn click_command(
    state: &BrowseState,
    screen_row: u16,
    total_rows: u16,
) -> Option<BrowseCommand> {
    if total_rows == 0 || screen_row == 0 || screen_row + 1 >= total_rows {
        return None;
    }
    let list_row = usize::from(screen_row.saturating_sub(1));
    let index = state.scroll + list_row;
    if index < state.rows.len() {
        Some(BrowseCommand::SelectIndex(index))
    } else {
        None
    }
}

pub fn parse_browse_input(prefix: &[u8]) -> Option<BrowseKey> {
    match prefix {
        [b'\r'] | [b'\n'] => Some(BrowseKey::Enter),
        [ESC] => Some(BrowseKey::Esc),
        [ESC, b'[', b'A', ..] => Some(BrowseKey::Up),
        [ESC, b'[', b'B', ..] => Some(BrowseKey::Down),
        [ESC, b'[', b'C', ..] => Some(BrowseKey::Right),
        [ESC, b'[', b'D', ..] => Some(BrowseKey::Left),
        [ESC, b'[', b'<', ..] => {
            let end = prefix.iter().position(|&b| b == b'M' || b == b'm')?;
            parse_sgr_mouse(&prefix[3..=end])
        }
        [byte] if byte.is_ascii_graphic() || *byte == b' ' => Some(BrowseKey::Char(*byte as char)),
        _ => None,
    }
}

/// Bytes of the first complete key/CSI sequence at the front of `input`.
fn first_input_len(input: &[u8]) -> Option<usize> {
    match input {
        [] => None,
        [ESC, b'[', rest @ ..] => rest
            .iter()
            .position(u8::is_ascii_alphabetic)
            .map(|i| 2 + i + 1),
        [ESC] => Some(1),
        [ESC, ..] => Some(1),
        _ => Some(1),
    }
}

fn parse_sgr_mouse(rest: &[u8]) -> Option<BrowseKey> {
    let (last, body) = rest.split_last()?;
    if *last != b'M' {
        return None;
    }
    let body = std::str::from_utf8(body).ok()?;
    let mut parts = body.split(';');
    let btn: u16 = parts.next()?.parse().ok()?;
    let col: u16 = parts.next()?.parse().ok()?;
    let row: u16 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    match btn {
        0 => Some(BrowseKey::MouseClick {
            row: row.saturating_sub(1),
            col: col.saturating_sub(1),
        }),
        64 => Some(BrowseKey::MouseWheelUp),
        65 => Some(BrowseKey::MouseWheelDown),
        _ => None,
    }
}

pub fn run_browse_overlay() -> Result<(), io::Error> {
    let start = std::env::var("HERDR_PREVIEW_BROWSE_START").map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "overlay env: HERDR_PREVIEW_BROWSE_START missing",
        )
    })?;
    let origin_cwd = std::env::var("HERDR_PREVIEW_BROWSE_CWD").map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "overlay env: HERDR_PREVIEW_BROWSE_CWD missing",
        )
    })?;
    let origin = std::env::var("HERDR_PREVIEW_BROWSE_ORIGIN").ok();

    let mut state = BrowseState::open(Path::new(&start));
    state.origin_cwd = Some(PathBuf::from(&origin_cwd));
    match browse_pick(&mut state)? {
        BrowseOutcome::OpenFile { path } => {
            crate::hint::open_preview_file(&path, Path::new(&origin_cwd), origin.as_deref())
                .map_err(|err| match err {
                    crate::hint::HintError::Io(e) => e,
                    other => io::Error::other(other.to_string()),
                })?;
            Ok(())
        }
        BrowseOutcome::Dismiss | BrowseOutcome::Continue => Ok(()),
    }
}

/// Run the browse TUI until dismiss or file pick. Drops RawMode/TerminalGuard
/// before returning so `open_preview_file` sees a restored TTY (same as hint overlay).
fn browse_pick(state: &mut BrowseState) -> io::Result<BrowseOutcome> {
    let mut stdout = io::stdout();
    let mut tty = open_tty()?;
    let _raw = RawMode::enter(tty.as_raw_fd())?;
    let _term = TerminalGuard::enter(&mut stdout)?;
    let mut leftover = Vec::new();

    loop {
        let (rows, cols) = tty_size(tty.as_raw_fd()).unwrap_or((24, 80));
        let visible_rows = (rows as usize).saturating_sub(2).max(1);
        paint_browse(&mut stdout, state, rows, cols)?;

        let key = match read_browse_key(&mut tty, &mut leftover)? {
            Some(key) => key,
            None => continue,
        };

        let outcome = match key {
            BrowseKey::MouseClick { row, .. } => {
                if let Some(cmd) = click_command(state, row, rows) {
                    let _ = state.apply(cmd, visible_rows);
                    state.apply(BrowseCommand::Activate, visible_rows)
                } else {
                    BrowseOutcome::Continue
                }
            }
            other => match map_browse_key(other) {
                Some(cmd) => state.apply(cmd, visible_rows),
                None => BrowseOutcome::Continue,
            },
        };

        match outcome {
            BrowseOutcome::Continue => {}
            other => return Ok(other),
        }
    }
}

fn paint_browse(out: &mut impl Write, state: &BrowseState, rows: u16, cols: u16) -> io::Result<()> {
    write!(out, "{ANSI_HOME}{ANSI_CLEAR}")?;
    let frame = render_browse(state, rows, cols);
    let mut lines = frame.split('\n').peekable();
    while let Some(line) = lines.next() {
        if lines.peek().is_some() {
            writeln!(out, "{line}{ANSI_CLR_EOL}")?;
        } else {
            write!(out, "{line}{ANSI_CLR_EOL}")?;
        }
    }
    out.flush()
}

fn truncate_chars(text: &str, cols: usize) -> String {
    text.chars().take(cols).collect()
}

/// RAII: restore cursor, wrap, and mouse when the overlay exits for any reason.
struct TerminalGuard;

impl TerminalGuard {
    fn enter(out: &mut impl Write) -> io::Result<Self> {
        write!(out, "{ANSI_HIDE}{ANSI_WRAP_OFF}{ANSI_MOUSE_ON}")?;
        out.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = write!(stdout, "{ANSI_MOUSE_OFF}{ANSI_SHOW}{ANSI_WRAP_ON}");
        let _ = stdout.flush();
    }
}

/// Put the pane TTY into non-canonical, no-echo mode so single keypresses
/// arrive immediately (same contract as hint overlay).
struct RawMode {
    fd: libc::c_int,
    original: libc::termios,
}

impl RawMode {
    fn enter(fd: libc::c_int) -> io::Result<Self> {
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, original })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

fn tty_size(fd: libc::c_int) -> io::Result<(u16, u16)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) != 0 {
            return Err(io::Error::last_os_error());
        }
        if ws.ws_row == 0 || ws.ws_col == 0 {
            return Err(io::Error::other("empty winsize"));
        }
        Ok((ws.ws_row, ws.ws_col))
    }
}

fn open_tty() -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
}

fn read_browse_key(tty: &mut fs::File, leftover: &mut Vec<u8>) -> io::Result<Option<BrowseKey>> {
    loop {
        if leftover.is_empty() {
            let mut buf = [0u8; 1];
            tty.read_exact(&mut buf)?;
            if buf[0] != ESC {
                leftover.push(buf[0]);
            } else {
                let fd = tty.as_raw_fd();
                let mut pollfd = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
                if ready == 0 {
                    return Ok(Some(BrowseKey::Esc));
                }
                if ready < 0 {
                    return Err(io::Error::last_os_error());
                }
                leftover.push(ESC);
                let mut extra = [0u8; 32];
                let n = tty.read(&mut extra)?;
                leftover.extend_from_slice(&extra[..n]);
            }
        }

        if let Some(n) = first_input_len(leftover) {
            let seq: Vec<u8> = leftover.drain(..n).collect();
            return Ok(parse_browse_input(&seq));
        }

        let mut extra = [0u8; 32];
        let n = tty.read(&mut extra)?;
        if n == 0 {
            leftover.clear();
            return Ok(None);
        }
        leftover.extend_from_slice(&extra[..n]);
    }
}
