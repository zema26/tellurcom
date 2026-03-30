use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use chrono::{DateTime, Local};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};

// ─── Color Palette ────────────────────────────────────────────────────────────
const COLOR_BG: Color = Color::Black;
const COLOR_PANEL_BG: Color = Color::Blue;
const COLOR_PANEL_BORDER: Color = Color::Cyan;
const COLOR_DIR: Color = Color::Cyan;
const COLOR_EXEC: Color = Color::Green;
const COLOR_LINK: Color = Color::Magenta;
const COLOR_SELECTED_BG: Color = Color::Cyan;
const COLOR_SELECTED_FG: Color = Color::Black;
const COLOR_INACTIVE_SELECTED_BG: Color = Color::DarkGray;
const COLOR_INACTIVE_SELECTED_FG: Color = Color::White;
const COLOR_STATUS_BG: Color = Color::Cyan;
const COLOR_STATUS_FG: Color = Color::Black;
const COLOR_FKEY_BG: Color = Color::Black;
const COLOR_FKEY_LABEL_BG: Color = Color::Cyan;
const COLOR_FKEY_LABEL_FG: Color = Color::Black;
const COLOR_FKEY_TEXT: Color = Color::White;
const COLOR_DIALOG_BG: Color = Color::Blue;
const COLOR_DIALOG_BORDER: Color = Color::Cyan;
const COLOR_ERROR: Color = Color::Red;
const COLOR_INFO: Color = Color::Yellow;
const COLOR_SIZE: Color = Color::Yellow;
const COLOR_DATE: Color = Color::Green;

// ─── File Entry ───────────────────────────────────────────────────────────────
#[derive(Clone, Debug)]
struct FileEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    is_symlink: bool,
    is_executable: bool,
    size: u64,
    permissions: u32,
    modified: Option<SystemTime>,
}

impl FileEntry {
    fn permissions_str(&self) -> String {
        let m = self.permissions;
        let file_type = if self.is_dir {
            'd'
        } else if self.is_symlink {
            'l'
        } else {
            '-'
        };
        format!(
            "{}{}{}{}{}{}{}{}{}{}",
            file_type,
            if m & 0o400 != 0 { 'r' } else { '-' },
            if m & 0o200 != 0 { 'w' } else { '-' },
            if m & 0o100 != 0 { 'x' } else { '-' },
            if m & 0o040 != 0 { 'r' } else { '-' },
            if m & 0o020 != 0 { 'w' } else { '-' },
            if m & 0o010 != 0 { 'x' } else { '-' },
            if m & 0o004 != 0 { 'r' } else { '-' },
            if m & 0o002 != 0 { 'w' } else { '-' },
            if m & 0o001 != 0 { 'x' } else { '-' },
        )
    }

    fn size_str(&self) -> String {
        if self.is_dir {
            return String::from("  <DIR>");
        }
        let s = self.size;
        if s >= 1_073_741_824 {
            format!("{:.1}G", s as f64 / 1_073_741_824.0)
        } else if s >= 1_048_576 {
            format!("{:.1}M", s as f64 / 1_048_576.0)
        } else if s >= 1_024 {
            format!("{:.1}K", s as f64 / 1_024.0)
        } else {
            format!("{}B", s)
        }
    }

    fn date_str(&self) -> String {
        match self.modified {
            Some(t) => {
                let dt: DateTime<Local> = t.into();
                dt.format("%Y-%m-%d %H:%M").to_string()
            }
            None => String::from("????-??-?? ??:??"),
        }
    }

    fn display_name(&self) -> String {
        if self.is_dir {
            format!("{}/", self.name)
        } else if self.is_symlink {
            format!("{}@", self.name)
        } else {
            self.name.clone()
        }
    }

    fn color(&self) -> Color {
        if self.is_symlink {
            COLOR_LINK
        } else if self.is_dir {
            COLOR_DIR
        } else if self.is_executable {
            COLOR_EXEC
        } else {
            Color::White
        }
    }
}

// ─── Panel ────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct Panel {
    path: PathBuf,
    entries: Vec<FileEntry>,
    state: ListState,
    scroll_offset: usize,
}

impl Panel {
    fn new(path: PathBuf) -> Self {
        let mut p = Panel {
            path,
            entries: Vec::new(),
            state: ListState::default(),
            scroll_offset: 0,
        };
        p.load_entries();
        p
    }

    fn load_entries(&mut self) {
        let mut entries = Vec::new();

        // Add parent dir entry unless we're at root
        if let Some(parent) = self.path.parent() {
            entries.push(FileEntry {
                name: String::from(".."),
                path: parent.to_path_buf(),
                is_dir: true,
                is_symlink: false,
                is_executable: false,
                size: 0,
                permissions: 0o755,
                modified: None,
            });
        }

        // Read directory contents
        let read = match fs::read_dir(&self.path) {
            Ok(r) => r,
            Err(_) => {
                self.entries = entries;
                if !self.entries.is_empty() {
                    self.state.select(Some(0));
                }
                return;
            }
        };

        let mut dir_entries: Vec<FileEntry> = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            let meta_result = entry.metadata();
            let symlink_meta_result = fs::symlink_metadata(&path);

            let is_symlink = symlink_meta_result
                .as_ref()
                .map(|m| m.is_symlink())
                .unwrap_or(false);

            let meta = match meta_result {
                Ok(m) => m,
                Err(_) => continue,
            };

            let is_dir = meta.is_dir();
            let size = if is_dir { 0 } else { meta.len() };
            let perms = meta.permissions().mode();
            let is_exec = !is_dir && (perms & 0o111 != 0);
            let modified = meta.modified().ok();
            let name = entry.file_name().to_string_lossy().into_owned();

            dir_entries.push(FileEntry {
                name,
                path,
                is_dir,
                is_symlink,
                is_executable: is_exec,
                size,
                permissions: perms,
                modified,
            });
        }

        // Sort: dirs first, then files, both alphabetically (case-insensitive)
        dir_entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        entries.extend(dir_entries);
        self.entries = entries;

        let selected = self.state.selected().unwrap_or(0);
        if !self.entries.is_empty() {
            self.state
                .select(Some(selected.min(self.entries.len() - 1)));
        } else {
            self.state.select(None);
        }
    }

    fn selected_entry(&self) -> Option<&FileEntry> {
        self.state.selected().and_then(|i| self.entries.get(i))
    }

    fn move_up(&mut self, _visible_height: usize) {
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => return,
        };
        self.state.select(Some(i));
        if i < self.scroll_offset {
            self.scroll_offset = i;
        }
    }

    fn move_down(&mut self, visible_height: usize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i + 1 < len => i + 1,
            _ => return,
        };
        self.state.select(Some(i));
        if i >= self.scroll_offset + visible_height {
            self.scroll_offset = i + 1 - visible_height;
        }
    }

    fn page_up(&mut self, visible_height: usize) {
        let i = self.state.selected().unwrap_or(0);
        let new_i = i.saturating_sub(visible_height);
        self.state.select(Some(new_i));
        self.scroll_offset = self.scroll_offset.saturating_sub(visible_height);
    }

    fn page_down(&mut self, visible_height: usize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().unwrap_or(0);
        let new_i = (i + visible_height).min(len - 1);
        self.state.select(Some(new_i));
        if new_i >= self.scroll_offset + visible_height {
            self.scroll_offset = (new_i + 1).saturating_sub(visible_height);
        }
    }

    fn go_home(&mut self) {
        if !self.entries.is_empty() {
            self.state.select(Some(0));
            self.scroll_offset = 0;
        }
    }

    fn go_end(&mut self) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        self.state.select(Some(len - 1));
    }

    fn enter_dir(&mut self, path: PathBuf) {
        self.path = path;
        self.scroll_offset = 0;
        self.state = ListState::default();
        self.load_entries();
        if !self.entries.is_empty() {
            self.state.select(Some(0));
        }
    }

    fn path_display(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

// ─── Dialog Kind ─────────────────────────────────────────────────────────────
#[derive(Clone, Debug)]
enum DialogKind {
    Delete { path: PathBuf, name: String },
    MkDir,
    Copy { src: PathBuf },
    Move { src: PathBuf },
    Rename { src: PathBuf, name: String },
    Info(String),
    Help,
}

#[derive(Clone)]
struct Dialog {
    kind: DialogKind,
    input: String,
    cursor: usize,
}

impl Dialog {
    fn new(kind: DialogKind, default_input: String) -> Self {
        let cursor = default_input.len();
        Dialog {
            kind,
            input: default_input,
            cursor,
        }
    }

    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            let before = &self.input[..self.cursor];
            let last_char_len = before.chars().last().map(|c| c.len_utf8()).unwrap_or(0);
            let new_cursor = self.cursor - last_char_len;
            self.input.remove(new_cursor);
            self.cursor = new_cursor;
        }
    }

    fn delete_char_after(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
        }
    }

    fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }
}

// ─── App State ────────────────────────────────────────────────────────────────
#[derive(PartialEq, Clone)]
enum ActivePane {
    Left,
    Right,
}

struct App {
    left: Panel,
    right: Panel,
    active: ActivePane,
    dialog: Option<Dialog>,
    status_msg: Option<(String, bool)>,
    panel_height: usize,
    quit: bool,
}

impl App {
    fn new() -> Self {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        let left = Panel::new(home);
        let right = Panel::new(PathBuf::from("/"));
        App {
            left,
            right,
            active: ActivePane::Left,
            dialog: None,
            status_msg: None,
            panel_height: 20,
            quit: false,
        }
    }

    fn active_panel(&self) -> &Panel {
        match self.active {
            ActivePane::Left => &self.left,
            ActivePane::Right => &self.right,
        }
    }

    fn active_panel_mut(&mut self) -> &mut Panel {
        match self.active {
            ActivePane::Left => &mut self.left,
            ActivePane::Right => &mut self.right,
        }
    }

    fn inactive_panel(&self) -> &Panel {
        match self.active {
            ActivePane::Left => &self.right,
            ActivePane::Right => &self.left,
        }
    }

    fn switch_pane(&mut self) {
        self.active = match self.active {
            ActivePane::Left => ActivePane::Right,
            ActivePane::Right => ActivePane::Left,
        };
    }

    fn set_status(&mut self, msg: String, is_error: bool) {
        self.status_msg = Some((msg, is_error));
    }

    fn clear_status(&mut self) {
        self.status_msg = None;
    }

    fn enter_selected(&mut self) {
        let entry = match self.active_panel().selected_entry() {
            Some(e) => e.clone(),
            None => return,
        };
        if entry.is_dir {
            let path = entry.path.clone();
            self.active_panel_mut().enter_dir(path);
            self.clear_status();
        }
    }

    fn go_parent(&mut self) {
        let parent = self.active_panel().path.parent().map(|p| p.to_path_buf());
        if let Some(p) = parent {
            self.active_panel_mut().enter_dir(p);
            self.clear_status();
        }
    }

    fn open_f3_view(&mut self) {
        let entry = match self.active_panel().selected_entry() {
            Some(e) => e.clone(),
            None => return,
        };
        if entry.is_dir {
            self.set_status(format!("'{}' is a directory", entry.name), false);
            return;
        }
        match fs::read_to_string(&entry.path) {
            Ok(content) => {
                let preview: String = content.chars().take(3000).collect();
                self.dialog = Some(Dialog::new(
                    DialogKind::Info(format!("─── {} ───\n\n{}", entry.name, preview)),
                    String::new(),
                ));
            }
            Err(e) => self.set_status(format!("Cannot read: {}", e), true),
        }
    }

    fn open_copy_dialog(&mut self) {
        let entry = match self.active_panel().selected_entry() {
            Some(e) if e.name != ".." => e.clone(),
            _ => {
                self.set_status("Cannot copy parent directory".to_string(), true);
                return;
            }
        };
        let dest = self
            .inactive_panel()
            .path
            .join(&entry.name)
            .to_string_lossy()
            .into_owned();
        self.dialog = Some(Dialog::new(DialogKind::Copy { src: entry.path }, dest));
    }

    fn open_move_dialog(&mut self) {
        let entry = match self.active_panel().selected_entry() {
            Some(e) if e.name != ".." => e.clone(),
            _ => {
                self.set_status("Cannot move parent directory".to_string(), true);
                return;
            }
        };
        let dest = self
            .inactive_panel()
            .path
            .join(&entry.name)
            .to_string_lossy()
            .into_owned();
        self.dialog = Some(Dialog::new(DialogKind::Move { src: entry.path }, dest));
    }

    fn open_mkdir_dialog(&mut self) {
        self.dialog = Some(Dialog::new(DialogKind::MkDir, String::new()));
    }

    fn open_delete_dialog(&mut self) {
        let entry = match self.active_panel().selected_entry() {
            Some(e) if e.name != ".." => e.clone(),
            _ => {
                self.set_status("Cannot delete parent directory".to_string(), true);
                return;
            }
        };
        self.dialog = Some(Dialog::new(
            DialogKind::Delete {
                path: entry.path,
                name: entry.name,
            },
            String::new(),
        ));
    }

    fn open_rename_dialog(&mut self) {
        let entry = match self.active_panel().selected_entry() {
            Some(e) if e.name != ".." => e.clone(),
            _ => {
                self.set_status("Cannot rename parent directory".to_string(), true);
                return;
            }
        };
        let name = entry.name.clone();
        self.dialog = Some(Dialog::new(
            DialogKind::Rename {
                src: entry.path,
                name: entry.name,
            },
            name,
        ));
    }

    fn do_mkdir(&mut self, name: &str) {
        if name.trim().is_empty() {
            self.set_status("Directory name cannot be empty".to_string(), true);
            return;
        }
        let new_dir = self.active_panel().path.join(name.trim());
        match fs::create_dir_all(&new_dir) {
            Ok(_) => {
                self.set_status(format!("Created: {}", name.trim()), false);
                self.active_panel_mut().load_entries();
            }
            Err(e) => self.set_status(format!("mkdir failed: {}", e), true),
        }
    }

    fn do_copy(&mut self, src: &Path, dest_str: &str) {
        if dest_str.trim().is_empty() {
            self.set_status("Destination cannot be empty".to_string(), true);
            return;
        }
        let dest = PathBuf::from(dest_str.trim());
        let dest = if dest.is_dir() {
            dest.join(src.file_name().unwrap_or_default())
        } else {
            dest
        };
        let result = if src.is_dir() {
            copy_dir_all(src, &dest)
        } else {
            fs::copy(src, &dest).map(|_| ())
        };
        match result {
            Ok(_) => {
                self.set_status(format!("Copied to: {}", dest.to_string_lossy()), false);
                self.left.load_entries();
                self.right.load_entries();
            }
            Err(e) => self.set_status(format!("Copy failed: {}", e), true),
        }
    }

    fn do_move(&mut self, src: &Path, dest_str: &str) {
        if dest_str.trim().is_empty() {
            self.set_status("Destination cannot be empty".to_string(), true);
            return;
        }
        let dest = PathBuf::from(dest_str.trim());
        let dest = if dest.is_dir() {
            dest.join(src.file_name().unwrap_or_default())
        } else {
            dest
        };
        match fs::rename(src, &dest) {
            Ok(_) => {
                self.set_status(format!("Moved to: {}", dest.to_string_lossy()), false);
                self.left.load_entries();
                self.right.load_entries();
            }
            Err(e) => self.set_status(format!("Move failed: {}", e), true),
        }
    }

    fn do_delete(&mut self, path: &Path, name: &str) {
        let result = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        match result {
            Ok(_) => {
                self.set_status(format!("Deleted: {}", name), false);
                self.active_panel_mut().load_entries();
            }
            Err(e) => self.set_status(format!("Delete failed: {}", e), true),
        }
    }

    fn do_rename(&mut self, src: &Path, new_name: &str) {
        if new_name.trim().is_empty() {
            self.set_status("Name cannot be empty".to_string(), true);
            return;
        }
        let parent = src.parent().unwrap_or(Path::new("."));
        let dest = parent.join(new_name.trim());
        match fs::rename(src, &dest) {
            Ok(_) => {
                self.set_status(format!("Renamed to: {}", new_name.trim()), false);
                self.active_panel_mut().load_entries();
            }
            Err(e) => self.set_status(format!("Rename failed: {}", e), true),
        }
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

// ─── UI Rendering ─────────────────────────────────────────────────────────────
fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Full background
    let bg = Block::default().style(Style::default().bg(COLOR_BG));
    f.render_widget(bg, area);

    // Main layout: top menu | panels | status | fkeys
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_menu_bar(f, vchunks[0]);

    let hchunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vchunks[1]);

    let panel_height = vchunks[1].height.saturating_sub(3) as usize;
    app.panel_height = panel_height;

    render_panel(f, hchunks[0], app, true);
    render_panel(f, hchunks[1], app, false);

    render_status_bar(f, vchunks[2], app);
    render_fkey_bar(f, vchunks[3]);

    // Dialog on top of everything
    if let Some(ref dialog) = app.dialog.clone() {
        render_dialog(f, area, dialog);
    }
}

fn render_menu_bar(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " ◈ TellurCom ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Left ",
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        Span::styled(
            " Files ",
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        Span::styled(
            " Commands ",
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        Span::styled(
            " Options ",
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        Span::styled(
            " Right ",
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        Span::styled(
            "                                              ",
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_panel(f: &mut Frame, area: Rect, app: &mut App, is_left: bool) {
    let is_active = (is_left && app.active == ActivePane::Left)
        || (!is_left && app.active == ActivePane::Right);

    let panel = if is_left { &app.left } else { &app.right };
    let path_str = panel.path_display();

    let border_style = if is_active {
        Style::default().fg(COLOR_PANEL_BORDER).bg(COLOR_PANEL_BG)
    } else {
        Style::default().fg(Color::DarkGray).bg(COLOR_PANEL_BG)
    };

    let title_style = if is_active {
        Style::default()
            .fg(Color::White)
            .bg(COLOR_PANEL_BG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray).bg(COLOR_PANEL_BG)
    };

    // Truncate path if too wide
    let max_title_len = (area.width as usize).saturating_sub(4);
    let title = if path_str.len() > max_title_len {
        format!("…{}", &path_str[path_str.len() - max_title_len + 1..])
    } else {
        path_str.clone()
    };

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(COLOR_PANEL_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    // Column header row
    let hdr_area = Rect { y: inner.y, height: 1, ..inner };
    let list_area = Rect {
        y: inner.y + 1,
        height: inner.height - 1,
        ..inner
    };

    let col_w = inner.width as usize;
    let date_w = 16usize;
    let size_w = 8usize;
    let name_w = col_w.saturating_sub(date_w + size_w + 2);

    let hdr_line = format!(
        "{:<nw$} {:>sw$} {:>dw$}",
        "Name",
        "Size",
        "Modified",
        nw = name_w,
        sw = size_w,
        dw = date_w
    );
    f.render_widget(
        Paragraph::new(hdr_line).style(
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        hdr_area,
    );

    // File list
    let visible_height = list_area.height as usize;
    let scroll_offset = if is_left {
        app.left.scroll_offset
    } else {
        app.right.scroll_offset
    };

    let entries = if is_left {
        &app.left.entries
    } else {
        &app.right.entries
    };
    let selected_idx = if is_left {
        app.left.state.selected().unwrap_or(0)
    } else {
        app.right.state.selected().unwrap_or(0)
    };

    let visible: Vec<(usize, &FileEntry)> = entries
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    let items: Vec<ListItem> = visible
        .iter()
        .map(|(real_idx, entry)| {
            let is_sel = *real_idx == selected_idx;
            let raw_name = entry.display_name();
            let name_display = if raw_name.chars().count() > name_w {
                let truncated: String = raw_name.chars().take(name_w.saturating_sub(1)).collect();
                format!("{}~", truncated)
            } else {
                raw_name
            };
            let size_s = entry.size_str();
            let date_s = entry.date_str();

            let line = if is_active && is_sel {
                Line::from(vec![
                    Span::styled(
                        format!("{:<nw$}", name_display, nw = name_w),
                        Style::default()
                            .fg(COLOR_SELECTED_FG)
                            .bg(COLOR_SELECTED_BG)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {:>sw$}", size_s, sw = size_w),
                        Style::default()
                            .fg(COLOR_SELECTED_FG)
                            .bg(COLOR_SELECTED_BG),
                    ),
                    Span::styled(
                        format!(" {:>dw$}", date_s, dw = date_w),
                        Style::default()
                            .fg(COLOR_SELECTED_FG)
                            .bg(COLOR_SELECTED_BG),
                    ),
                ])
            } else if !is_active && is_sel {
                Line::from(vec![Span::styled(
                    format!(
                        "{:<nw$} {:>sw$} {:>dw$}",
                        name_display,
                        size_s,
                        date_s,
                        nw = name_w,
                        sw = size_w,
                        dw = date_w
                    ),
                    Style::default()
                        .fg(COLOR_INACTIVE_SELECTED_FG)
                        .bg(COLOR_INACTIVE_SELECTED_BG),
                )])
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("{:<nw$}", name_display, nw = name_w),
                        Style::default().fg(entry.color()).bg(COLOR_PANEL_BG),
                    ),
                    Span::styled(
                        format!(" {:>sw$}", size_s, sw = size_w),
                        Style::default().fg(COLOR_SIZE).bg(COLOR_PANEL_BG),
                    ),
                    Span::styled(
                        format!(" {:>dw$}", date_s, dw = date_w),
                        Style::default().fg(COLOR_DATE).bg(COLOR_PANEL_BG),
                    ),
                ])
            };
            ListItem::new(line)
        })
        .collect();

    f.render_widget(
        List::new(items).style(Style::default().bg(COLOR_PANEL_BG)),
        list_area,
    );

    // Scrollbar
    let total = entries.len();
    if total > visible_height && list_area.width > 0 && list_area.height > 0 {
        let bar_h = list_area.height as usize;
        let thumb_size = (visible_height * bar_h / total).max(1).min(bar_h);
        let thumb_pos = if total > visible_height {
            scroll_offset * (bar_h - thumb_size) / (total - visible_height)
        } else {
            0
        };
        let sb_x = list_area.x + list_area.width - 1;
        for row in 0..bar_h {
            let ch = if row >= thumb_pos && row < thumb_pos + thumb_size {
                "█"
            } else {
                "░"
            };
            let cell = Rect {
                x: sb_x,
                y: list_area.y + row as u16,
                width: 1,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(ch)
                    .style(Style::default().fg(Color::Cyan).bg(COLOR_PANEL_BG)),
                cell,
            );
        }
    }
}

fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let content = match &app.status_msg {
        Some((msg, is_error)) => {
            let color = if *is_error { COLOR_ERROR } else { COLOR_INFO };
            Line::from(vec![Span::styled(
                format!(" ● {} ", msg),
                Style::default()
                    .fg(color)
                    .bg(COLOR_STATUS_BG)
                    .add_modifier(Modifier::BOLD),
            )])
        }
        None => {
            let panel = app.active_panel();
            let total = panel.entries.len();
            let sel_n = panel.state.selected().map(|i| i + 1).unwrap_or(0);
            let entry = panel.selected_entry();

            let (perms, size_s, name_s) = match entry {
                Some(e) => (
                    e.permissions_str(),
                    e.size_str(),
                    format!(" {}", e.display_name()),
                ),
                None => (
                    String::from("----------"),
                    String::from("      0"),
                    String::new(),
                ),
            };

            Line::from(vec![
                Span::styled(
                    format!(" {}/{} ", sel_n, total),
                    Style::default()
                        .fg(COLOR_STATUS_FG)
                        .bg(COLOR_STATUS_BG)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} ", perms),
                    Style::default().fg(Color::DarkGray).bg(COLOR_STATUS_BG),
                ),
                Span::styled(
                    format!("{} ", size_s),
                    Style::default().fg(Color::Black).bg(COLOR_STATUS_BG),
                ),
                Span::styled(
                    name_s,
                    Style::default()
                        .fg(COLOR_STATUS_FG)
                        .bg(COLOR_STATUS_BG)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        }
    };

    f.render_widget(
        Paragraph::new(content).style(Style::default().bg(COLOR_STATUS_BG)),
        area,
    );
}

fn render_fkey_bar(f: &mut Frame, area: Rect) {
    let keys: &[(&str, &str)] = &[
        ("1", "Help"),
        ("3", "View"),
        ("4", "Edit"),
        ("5", "Copy"),
        ("6", "Move"),
        ("7", "MkDir"),
        ("8", "Delete"),
        ("9", "Rename"),
        ("10", "Quit"),
    ];

    let mut spans = Vec::new();
    for (key, label) in keys {
        spans.push(Span::styled(
            format!("F{}", key),
            Style::default()
                .fg(COLOR_FKEY_LABEL_FG)
                .bg(COLOR_FKEY_LABEL_BG)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{} ", label),
            Style::default().fg(COLOR_FKEY_TEXT).bg(COLOR_FKEY_BG),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(COLOR_FKEY_BG)),
        area,
    );
}

fn render_dialog(f: &mut Frame, area: Rect, dialog: &Dialog) {
    let (title, body_text, show_input, show_confirm, show_ok) = match &dialog.kind {
        DialogKind::Delete { name, .. } => (
            " ⚠ Delete Confirmation ",
            format!(
                "Are you sure you want to delete:\n\n  {}\n\nThis cannot be undone!",
                name
            ),
            false,
            true,
            false,
        ),
        DialogKind::MkDir => (
            " Make Directory ",
            String::from("New directory name:"),
            true,
            false,
            false,
        ),
        DialogKind::Copy { src } => (
            " Copy ",
            format!(
                "Copy: {}\n\nDestination:",
                src.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ),
            true,
            false,
            false,
        ),
        DialogKind::Move { src } => (
            " Move / Rename ",
            format!(
                "Move: {}\n\nDestination:",
                src.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ),
            true,
            false,
            false,
        ),
        DialogKind::Rename { name, .. } => (
            " Rename ",
            format!("Rename: {}\n\nNew name:", name),
            true,
            false,
            false,
        ),
        DialogKind::Info(text) => {
            (" View ", text.clone(), false, false, true)
        }
        DialogKind::Help => (
            " TellurCom Help ",
            [
                "Navigation:",
                "  ↑↓ / j k     Move cursor",
                "  PgUp / PgDn  Page up/down",
                "  Home / End   First/last entry",
                "  Enter        Open directory",
                "  Backspace    Go to parent",
                "  Tab          Switch panel",
                "  ~            Go to home directory",
                "  /            Go to root",
                "",
                "File Operations:",
                "  F3    View file contents",
                "  F5    Copy file or directory",
                "  F6    Move file or directory",
                "  F7    Create directory",
                "  F8    Delete file or directory",
                "  F9    Rename file or directory",
                "",
                "  F1    This help screen",
                "  F10 / q / Ctrl+C   Quit",
            ]
            .join("\n"),
            false,
            false,
            true,
        ),
    };

    let body_lines: Vec<&str> = body_text.lines().collect();
    let content_height = body_lines.len() as u16;
    let input_height: u16 = if show_input { 1 } else { 0 };
    let btn_height: u16 = if show_confirm || show_ok { 2 } else { 1 };
    let dialog_height = content_height + input_height + btn_height + 2 + 1;
    let dialog_height = dialog_height.min(area.height.saturating_sub(2));
    let dialog_width = 64u16.min(area.width.saturating_sub(4));

    let dx = area.x + area.width.saturating_sub(dialog_width) / 2;
    let dy = area.y + area.height.saturating_sub(dialog_height) / 2;

    let dialog_area = Rect {
        x: dx,
        y: dy,
        width: dialog_width,
        height: dialog_height,
    };

    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .title_style(
            Style::default()
                .fg(Color::White)
                .bg(COLOR_DIALOG_BG)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_DIALOG_BORDER).bg(COLOR_DIALOG_BG))
        .style(Style::default().bg(COLOR_DIALOG_BG));

    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 2,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(4),
        height: dialog_area.height.saturating_sub(2),
    };

    let mut y = 0u16;

    // Body lines
    for line in &body_lines {
        if y >= inner.height {
            break;
        }
        f.render_widget(
            Paragraph::new(*line).style(Style::default().fg(Color::White).bg(COLOR_DIALOG_BG)),
            Rect {
                x: inner.x,
                y: inner.y + y,
                width: inner.width,
                height: 1,
            },
        );
        y += 1;
    }

    // Input field
    if show_input {
        if y < inner.height {
            let input_rect = Rect {
                x: inner.x,
                y: inner.y + y,
                width: inner.width,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(dialog.input.as_str()).style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                input_rect,
            );
            let cur_x = (inner.x + dialog.cursor.min(inner.width as usize - 1) as u16)
                .min(inner.x + inner.width - 1);
            f.set_cursor_position((cur_x, inner.y + y));
            y += 1;
        }
    }

    y += 1; // blank line before buttons

    // Buttons
    if y < inner.height {
        let btn_rect = Rect {
            x: inner.x,
            y: inner.y + y,
            width: inner.width,
            height: 1,
        };
        if show_confirm {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        " [Enter] Yes, Delete ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Red)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("   "),
                    Span::styled(
                        " [Esc] Cancel ",
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])),
                btn_rect,
            );
        } else if show_ok {
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    " [Esc / Enter / any key] Close ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )])),
                btn_rect,
            );
        } else {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        " [Enter] OK ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("   "),
                    Span::styled(
                        " [Esc] Cancel ",
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])),
                btn_rect,
            );
        }
    }
}

// ─── Event Handling ───────────────────────────────────────────────────────────
fn handle_event(app: &mut App, key: crossterm::event::KeyEvent) {
    // ── Dialog mode ──
    if app.dialog.is_some() {
        let kind = app.dialog.as_ref().unwrap().kind.clone();

        match kind {
            DialogKind::Info(_) | DialogKind::Help => {
                app.dialog = None;
            }

            DialogKind::Delete { path, name } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    app.dialog = None;
                    let p = path.clone();
                    let n = name.clone();
                    app.do_delete(&p, &n);
                }
                _ => {
                    app.dialog = None;
                }
            },

            DialogKind::MkDir => match key.code {
                KeyCode::Enter => {
                    let name = app.dialog.as_ref().unwrap().input.clone();
                    app.dialog = None;
                    app.do_mkdir(&name);
                }
                KeyCode::Esc => {
                    app.dialog = None;
                }
                KeyCode::Char(c) => {
                    if let Some(d) = &mut app.dialog {
                        d.insert_char(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_before();
                    }
                }
                KeyCode::Delete => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_after();
                    }
                }
                KeyCode::Left => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_left();
                    }
                }
                KeyCode::Right => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_right();
                    }
                }
                KeyCode::Home => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor = 0;
                    }
                }
                KeyCode::End => {
                    if let Some(d) = &mut app.dialog {
                        let len = d.input.len();
                        d.cursor = len;
                    }
                }
                _ => {}
            },

            DialogKind::Copy { src } => match key.code {
                KeyCode::Enter => {
                    let dest = app.dialog.as_ref().unwrap().input.clone();
                    app.dialog = None;
                    let s = src.clone();
                    app.do_copy(&s, &dest);
                }
                KeyCode::Esc => {
                    app.dialog = None;
                }
                KeyCode::Char(c) => {
                    if let Some(d) = &mut app.dialog {
                        d.insert_char(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_before();
                    }
                }
                KeyCode::Delete => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_after();
                    }
                }
                KeyCode::Left => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_left();
                    }
                }
                KeyCode::Right => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_right();
                    }
                }
                KeyCode::Home => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor = 0;
                    }
                }
                KeyCode::End => {
                    if let Some(d) = &mut app.dialog {
                        let len = d.input.len();
                        d.cursor = len;
                    }
                }
                _ => {}
            },

            DialogKind::Move { src } => match key.code {
                KeyCode::Enter => {
                    let dest = app.dialog.as_ref().unwrap().input.clone();
                    app.dialog = None;
                    let s = src.clone();
                    app.do_move(&s, &dest);
                }
                KeyCode::Esc => {
                    app.dialog = None;
                }
                KeyCode::Char(c) => {
                    if let Some(d) = &mut app.dialog {
                        d.insert_char(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_before();
                    }
                }
                KeyCode::Delete => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_after();
                    }
                }
                KeyCode::Left => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_left();
                    }
                }
                KeyCode::Right => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_right();
                    }
                }
                KeyCode::Home => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor = 0;
                    }
                }
                KeyCode::End => {
                    if let Some(d) = &mut app.dialog {
                        let len = d.input.len();
                        d.cursor = len;
                    }
                }
                _ => {}
            },

            DialogKind::Rename { src, .. } => match key.code {
                KeyCode::Enter => {
                    let new_name = app.dialog.as_ref().unwrap().input.clone();
                    app.dialog = None;
                    let s = src.clone();
                    app.do_rename(&s, &new_name);
                }
                KeyCode::Esc => {
                    app.dialog = None;
                }
                KeyCode::Char(c) => {
                    if let Some(d) = &mut app.dialog {
                        d.insert_char(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_before();
                    }
                }
                KeyCode::Delete => {
                    if let Some(d) = &mut app.dialog {
                        d.delete_char_after();
                    }
                }
                KeyCode::Left => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_left();
                    }
                }
                KeyCode::Right => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor_right();
                    }
                }
                KeyCode::Home => {
                    if let Some(d) = &mut app.dialog {
                        d.cursor = 0;
                    }
                }
                KeyCode::End => {
                    if let Some(d) = &mut app.dialog {
                        let len = d.input.len();
                        d.cursor = len;
                    }
                }
                _ => {}
            },
        }
        return;
    }

    // ── Normal mode ──
    let ph = app.panel_height.max(1);

    match key.code {
        // Quit
        KeyCode::F(10) | KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit = true,

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => {
            app.active_panel_mut().move_up(ph);
            app.clear_status();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.active_panel_mut().move_down(ph);
            app.clear_status();
        }
        KeyCode::PageUp => {
            app.active_panel_mut().page_up(ph);
            app.clear_status();
        }
        KeyCode::PageDown => {
            app.active_panel_mut().page_down(ph);
            app.clear_status();
        }
        KeyCode::Home => {
            app.active_panel_mut().go_home();
            app.clear_status();
        }
        KeyCode::End => {
            app.active_panel_mut().go_end();
            app.clear_status();
        }

        // Switch panels
        KeyCode::Tab => {
            app.switch_pane();
            app.clear_status();
        }

        // Enter selected directory
        KeyCode::Enter | KeyCode::Right => {
            app.enter_selected();
        }

        // Parent directory
        KeyCode::Backspace | KeyCode::Left => {
            app.go_parent();
        }

        // Refresh
        KeyCode::Char('R') => {
            app.left.load_entries();
            app.right.load_entries();
            app.set_status("Refreshed".to_string(), false);
        }

        // Go to home
        KeyCode::Char('~') => {
            let home = std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/"));
            app.active_panel_mut().enter_dir(home);
            app.clear_status();
        }

        // Go to root
        KeyCode::Char('\\') => {
            app.active_panel_mut().enter_dir(PathBuf::from("/"));
            app.clear_status();
        }

        // Help
        KeyCode::F(1) => {
            app.dialog = Some(Dialog::new(DialogKind::Help, String::new()));
        }

        // View
        KeyCode::F(3) => {
            app.open_f3_view();
        }

        // Copy
        KeyCode::F(5) => {
            app.open_copy_dialog();
        }

        // Move
        KeyCode::F(6) => {
            app.open_move_dialog();
        }

        // MkDir
        KeyCode::F(7) => {
            app.open_mkdir_dialog();
        }

        // Delete
        KeyCode::F(8) => {
            app.open_delete_dialog();
        }

        // Rename
        KeyCode::F(9) => {
            app.open_rename_dialog();
        }

        _ => {}
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────
fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("TellurCom error: {}", e);
    }

    println!("\n◈ TellurCom closed. Goodbye!\n");
    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| render(f, app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    handle_event(app, key);
                }
            }
        }

        if app.quit {
            return Ok(());
        }
    }
}
