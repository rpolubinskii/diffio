use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
};

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::{
    diff::Diff,
    git::{FileEntry, View, display_path},
    worker::{Request, Response},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    #[default]
    Auto,
    Split,
    Unified,
}

impl LayoutMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Split => "split",
            Self::Unified => "unified",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Diff,
}

#[derive(Clone, Debug)]
pub struct TreeRow {
    pub path: PathBuf,
    pub depth: usize,
    pub directory: bool,
    pub file_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PositionInFile {
    pub vertical: usize,
    pub horizontal: u16,
}

pub struct App {
    pub root: PathBuf,
    pub files: Vec<FileEntry>,
    pub selected: Option<PathBuf>,
    pub tree_cursor: Option<PathBuf>,
    pub collapsed: BTreeSet<PathBuf>,
    pub tree_offset: usize,
    pub diff: Option<Diff>,
    pub view: View,
    pub layout: LayoutMode,
    pub show_tree: bool,
    pub focus: Focus,
    pub position: PositionInFile,
    pub generation: u64,
    pub refresh_requested: bool,
    pub error: Option<String>,
    pub help: bool,
    pub search_input: Option<String>,
    pub search: String,
    pub notice: Option<String>,
    pub quit: bool,
    pub tree_area: Rect,
    pub diff_area: Rect,
    pub split: bool,
    positions: HashMap<(View, PathBuf), PositionInFile>,
    initially_selected: bool,
}

impl App {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: Vec::new(),
            selected: None,
            tree_cursor: None,
            collapsed: BTreeSet::new(),
            tree_offset: 0,
            diff: None,
            view: View::All,
            layout: LayoutMode::Auto,
            show_tree: true,
            focus: Focus::Tree,
            position: PositionInFile::default(),
            generation: 0,
            refresh_requested: true,
            error: None,
            help: false,
            search_input: None,
            search: String::new(),
            notice: None,
            quit: false,
            tree_area: Rect::default(),
            diff_area: Rect::default(),
            split: false,
            positions: HashMap::new(),
            initially_selected: false,
        }
    }

    pub fn request(&self) -> Request {
        Request {
            generation: self.generation,
            view: self.view,
            selected: self.selected.clone(),
        }
    }

    pub fn apply(&mut self, response: Response) -> bool {
        if response.request.generation != self.generation
            || response.request.view != self.view
            || response.request.selected != self.selected
        {
            self.refresh_requested = true;
            return false;
        }
        match response.result {
            Ok(snapshot) => {
                self.error = None;
                self.files = snapshot.files;
                self.diff = snapshot.diff;
                if !self.initially_selected {
                    if let Some(file) = self.files.first() {
                        let path = file.path.clone();
                        self.tree_cursor = Some(path.clone());
                        self.select(path);
                    }
                } else if !self
                    .tree_rows()
                    .iter()
                    .any(|row| Some(&row.path) == self.tree_cursor.as_ref())
                {
                    self.tree_cursor = self.tree_rows().first().map(|row| row.path.clone());
                }
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    fn save_position(&mut self) {
        if let Some(path) = &self.selected {
            self.positions
                .insert((self.view, path.clone()), self.position);
        }
    }

    pub fn select(&mut self, path: PathBuf) {
        if self.selected.as_ref() == Some(&path) {
            return;
        }
        self.save_position();
        self.position = self
            .positions
            .get(&(self.view, path.clone()))
            .copied()
            .unwrap_or_default();
        self.selected = Some(path);
        self.initially_selected = true;
        self.selection_changed();
    }

    pub fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.save_position();
        self.view = view;
        self.position = self
            .selected
            .as_ref()
            .and_then(|path| self.positions.get(&(view, path.clone())))
            .copied()
            .unwrap_or_default();
        self.files.clear();
        self.selection_changed();
    }

    fn selection_changed(&mut self) {
        self.generation += 1;
        self.diff = None;
        self.notice = None;
        self.refresh_requested = true;
    }

    pub fn tree_rows(&self) -> Vec<TreeRow> {
        let mut rows = BTreeMap::new();
        for (file_index, file) in self.files.iter().enumerate() {
            for ancestor in file
                .path
                .ancestors()
                .skip(1)
                .filter(|p| !p.as_os_str().is_empty())
            {
                rows.entry(ancestor.to_path_buf()).or_insert(TreeRow {
                    path: ancestor.to_path_buf(),
                    depth: ancestor.components().count() - 1,
                    directory: true,
                    file_index: None,
                });
            }
            rows.insert(
                file.path.clone(),
                TreeRow {
                    path: file.path.clone(),
                    depth: file.path.components().count() - 1,
                    directory: false,
                    file_index: Some(file_index),
                },
            );
        }
        rows.into_values()
            .filter(|row| {
                !row.path
                    .ancestors()
                    .skip(1)
                    .any(|p| self.collapsed.contains(p))
            })
            .collect()
    }

    fn move_tree(&mut self, delta: isize) {
        let rows = self.tree_rows();
        if rows.is_empty() {
            return;
        }
        let current = rows
            .iter()
            .position(|row| Some(&row.path) == self.tree_cursor.as_ref())
            .unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(rows.len() - 1);
        self.tree_cursor = Some(rows[next].path.clone());
        if !rows[next].directory {
            self.select(rows[next].path.clone());
        }
    }

    fn open_tree(&mut self, toggle: bool) {
        let rows = self.tree_rows();
        let Some(row) = rows
            .iter()
            .find(|row| Some(&row.path) == self.tree_cursor.as_ref())
        else {
            return;
        };
        if row.directory {
            if !self.collapsed.remove(&row.path) {
                if toggle {
                    self.collapsed.insert(row.path.clone());
                } else {
                    self.move_tree(1);
                }
            }
        } else {
            self.select(row.path.clone());
            self.focus = Focus::Diff;
        }
    }

    fn close_tree(&mut self) {
        let rows = self.tree_rows();
        if let Some(row) = rows
            .iter()
            .find(|row| Some(&row.path) == self.tree_cursor.as_ref())
        {
            if row.directory && !self.collapsed.contains(&row.path) {
                self.collapsed.insert(row.path.clone());
            } else if let Some(parent) = row.path.parent().filter(|p| !p.as_os_str().is_empty()) {
                self.tree_cursor = Some(parent.to_path_buf());
            }
        }
    }

    pub fn scroll(&mut self, delta: isize) {
        let length = self.diff.as_ref().map_or(0, |diff| diff.len(self.split));
        self.position.vertical = self
            .position
            .vertical
            .saturating_add_signed(delta)
            .min(length.saturating_sub(1));
    }

    pub fn update_split(&mut self, split: bool) {
        if split == self.split {
            return;
        }
        if let Some(diff) = &self.diff {
            let anchor = diff
                .row_indices(self.position.vertical, self.split)
                .first()
                .copied();
            if let Some(anchor) = anchor {
                self.position.vertical = if split {
                    diff.pairs
                        .iter()
                        .position(|pair| pair.left == Some(anchor) || pair.right == Some(anchor))
                        .unwrap_or(0)
                } else {
                    anchor
                };
            }
        }
        self.split = split;
    }

    fn hunk(&mut self, forward: bool) {
        if let Some(diff) = &self.diff {
            let positions = (0..diff.len(self.split)).filter(|&i| diff.is_hunk(i, self.split));
            let next = if forward {
                positions.filter(|&i| i > self.position.vertical).min()
            } else {
                positions.filter(|&i| i < self.position.vertical).max()
            };
            if let Some(next) = next {
                self.position.vertical = next;
            }
        }
    }

    fn find(&mut self, forward: bool, include_current: bool) {
        if self.search.is_empty() {
            return;
        }
        let query = self.search.to_ascii_lowercase();
        if self.focus == Focus::Tree {
            let matches: Vec<_> = self
                .files
                .iter()
                .filter(|file| {
                    display_path(&file.path)
                        .to_ascii_lowercase()
                        .contains(&query)
                })
                .map(|file| file.path.clone())
                .collect();
            if matches.is_empty() {
                self.notice = Some("No matching file".into());
                return;
            }
            let current = matches
                .iter()
                .position(|path| Some(path) == self.selected.as_ref());
            let next = match current {
                Some(i) if include_current => i,
                Some(i) if forward => (i + 1) % matches.len(),
                Some(i) => (i + matches.len() - 1) % matches.len(),
                None => 0,
            };
            let path = matches[next].clone();
            for ancestor in path.ancestors().skip(1) {
                self.collapsed.remove(ancestor);
            }
            self.tree_cursor = Some(path.clone());
            self.select(path);
        } else if let Some(diff) = &self.diff {
            let length = diff.len(self.split);
            let start = usize::from(!include_current);
            for distance in start..length + start {
                let index = if forward {
                    (self.position.vertical + distance) % length
                } else {
                    (self.position.vertical + length - distance % length) % length
                };
                if diff
                    .row_indices(index, self.split)
                    .iter()
                    .any(|&i| diff.rows[i].text.to_ascii_lowercase().contains(&query))
                {
                    self.position.vertical = index;
                    self.notice = None;
                    return;
                }
            }
            self.notice = Some("No matching text in this diff".into());
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(input) = &mut self.search_input {
            match key.code {
                KeyCode::Esc => self.search_input = None,
                KeyCode::Enter => {
                    self.search = self.search_input.take().unwrap_or_default();
                    self.find(true, true);
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    input.push(c)
                }
                _ => {}
            }
            return;
        }
        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?' | 'q') | KeyCode::Enter
            ) {
                self.help = false;
            }
            return;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return;
        }
        self.notice = None;
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('r') => self.refresh_requested = true,
            KeyCode::Char('1') => self.set_view(View::All),
            KeyCode::Char('2') => self.set_view(View::Staged),
            KeyCode::Char('3') => self.set_view(View::Unstaged),
            KeyCode::Char('v') => {
                self.layout = match self.layout {
                    LayoutMode::Auto => LayoutMode::Split,
                    LayoutMode::Split => LayoutMode::Unified,
                    LayoutMode::Unified => LayoutMode::Auto,
                }
            }
            KeyCode::Char('t') => {
                self.show_tree = !self.show_tree;
                if !self.show_tree {
                    self.focus = Focus::Diff;
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.show_tree && self.focus == Focus::Diff {
                    Focus::Tree
                } else {
                    Focus::Diff
                };
            }
            KeyCode::Char('/') => self.search_input = Some(String::new()),
            KeyCode::Char('n') => self.find(true, false),
            KeyCode::Char('N') => self.find(false, false),
            KeyCode::Char(']') => self.hunk(true),
            KeyCode::Char('[') => self.hunk(false),
            KeyCode::Down | KeyCode::Char('j') => {
                if self.focus == Focus::Tree {
                    self.move_tree(1);
                } else {
                    self.scroll(1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.focus == Focus::Tree {
                    self.move_tree(-1);
                } else {
                    self.scroll(-1);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.focus == Focus::Tree {
                    self.close_tree();
                } else {
                    self.position.horizontal = self.position.horizontal.saturating_sub(4);
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.focus == Focus::Tree {
                    self.open_tree(false);
                } else {
                    self.position.horizontal = self.position.horizontal.saturating_add(4);
                }
            }
            KeyCode::Enter if self.focus == Focus::Tree => self.open_tree(true),
            KeyCode::PageDown => {
                if self.focus == Focus::Tree {
                    self.move_tree(self.tree_area.height as isize);
                } else {
                    self.scroll(self.diff_area.height as isize);
                }
            }
            KeyCode::PageUp => {
                if self.focus == Focus::Tree {
                    self.move_tree(-(self.tree_area.height as isize));
                } else {
                    self.scroll(-(self.diff_area.height as isize));
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if self.focus == Focus::Tree {
                    self.move_tree(-(self.tree_rows().len() as isize));
                } else {
                    self.position.vertical = 0;
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.focus == Focus::Tree {
                    self.move_tree(self.tree_rows().len() as isize);
                } else {
                    self.scroll(isize::MAX);
                }
            }
            _ => {}
        }
    }

    pub fn mouse(&mut self, mouse: MouseEvent) {
        if self.help || self.search_input.is_some() {
            return;
        }
        let position = Position::new(mouse.column, mouse.row);
        if self.tree_area.contains(position) {
            self.focus = Focus::Tree;
            match mouse.kind {
                MouseEventKind::ScrollDown => self.move_tree(3),
                MouseEventKind::ScrollUp => self.move_tree(-3),
                MouseEventKind::Down(MouseButton::Left) => {
                    let index =
                        self.tree_offset + mouse.row.saturating_sub(self.tree_area.y) as usize;
                    if let Some(row) = self.tree_rows().get(index) {
                        self.tree_cursor = Some(row.path.clone());
                        self.open_tree(true);
                    }
                }
                _ => {}
            }
        } else if self.diff_area.contains(position) {
            self.focus = Focus::Diff;
            match mouse.kind {
                MouseEventKind::ScrollDown => self.scroll(3),
                MouseEventKind::ScrollUp => self.scroll(-3),
                MouseEventKind::ScrollLeft => {
                    self.position.horizontal = self.position.horizontal.saturating_sub(4)
                }
                MouseEventKind::ScrollRight => {
                    self.position.horizontal = self.position.horizontal.saturating_add(4)
                }
                _ => {}
            }
        }
    }
}
