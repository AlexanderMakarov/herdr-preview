use std::fs;
use std::io;
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
            },
            Err(_) => Self {
                cwd: cwd.clone(),
                rows: fallback_rows(&cwd),
                selected: 0,
                scroll: 0,
                notice: Some("cannot read directory".into()),
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
                Some(BrowseRow::File { path, .. }) => BrowseOutcome::OpenFile {
                    path: path.clone(),
                },
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
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_path = entry.path();
        if file_type.is_dir() {
            children.push((true, name, child_path));
        } else if file_type.is_file() {
            children.push((false, name, child_path));
        }
    }

    children.sort_by(|(a_is_dir, a_name, _), (b_is_dir, b_name, _)| {
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a_name.to_lowercase().cmp(&b_name.to_lowercase()),
        }
    });

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
