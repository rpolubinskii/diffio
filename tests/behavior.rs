use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use diffio::{
    app::{App, Focus, LayoutMode},
    diff::{Diff, Kind},
    git::{FileEntry, Repository, View},
    ui,
    worker::{Response, Worker, refresh},
};
use ratatui::{Terminal, backend::TestBackend};
use tempfile::TempDir;

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        fixture.git(&["init", "-q", "-b", "main"]);
        fixture.git(&["config", "user.email", "test@example.invalid"]);
        fixture.git(&["config", "user.name", "Test"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(&self, path: impl AsRef<Path>, content: impl AsRef<[u8]>) {
        let path = self.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "fixture"]);
    }

    fn repo(&self) -> Repository {
        Repository::discover(self.path()).unwrap()
    }
}

fn file<'a>(files: &'a [FileEntry], path: &str) -> &'a FileEntry {
    files
        .iter()
        .find(|file| file.path == Path::new(path))
        .unwrap()
}

fn has(diff: &Diff, kind: Kind, text: &str) -> bool {
    diff.rows
        .iter()
        .any(|row| row.kind == kind && row.text == text)
}

fn refresh_app(repo: &Repository, app: &mut App) {
    let request = app.request();
    let result = refresh(repo, &request);
    assert!(app.apply(Response { request, result }));
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn repository_views_include_changes_and_respect_ignore_rules() {
    let fixture = Fixture::new();
    for name in ["modified", "deleted", "old name", "staged"] {
        fixture.write(name, "base\n");
    }
    fixture.write(".gitignore", "ignored*\n");
    fixture.commit();
    fixture.write("modified", "working\n");
    fixture.write("staged", "index\n");
    fixture.git(&["add", "staged"]);
    fs::remove_file(fixture.path().join("deleted")).unwrap();
    fixture.git(&["mv", "old name", "renamed name"]);
    fixture.write("dir/untracked", "new\n");
    fixture.write("ignored.log", "ignore\n");
    let repo = fixture.repo();
    let index_path = fixture.path().join(".git/index");
    let index_before = fs::read(&index_path).unwrap();
    let index_modified = fs::metadata(&index_path).unwrap().modified().unwrap();
    let files = repo.status().unwrap();
    assert_eq!(files.len(), 5);
    assert!(file(&files, "dir/untracked").untracked());
    assert_eq!(
        file(&files, "renamed name").original.as_deref(),
        Some(Path::new("old name"))
    );
    assert_eq!(files.iter().filter(|f| f.visible(View::Staged)).count(), 2);
    assert_eq!(
        files.iter().filter(|f| f.visible(View::Unstaged)).count(),
        3
    );
    assert!(has(
        &repo.diff(file(&files, "modified"), View::All).unwrap(),
        Kind::Add,
        "working"
    ));
    assert!(has(
        &repo.diff(file(&files, "deleted"), View::All).unwrap(),
        Kind::Delete,
        "base"
    ));
    assert!(
        repo.diff(file(&files, "renamed name"), View::Staged)
            .unwrap()
            .rows
            .iter()
            .any(|row| row.text.contains("rename to renamed name"))
    );
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(
        fs::metadata(index_path).unwrap().modified().unwrap(),
        index_modified
    );
}

#[test]
fn all_compares_head_to_working_content_and_retains_cancelled_staging() {
    let fixture = Fixture::new();
    fixture.write("file", "head\n");
    fixture.commit();
    fixture.write("file", "staged\n");
    fixture.git(&["add", "file"]);
    fixture.write("file", "head\n");
    let repo = fixture.repo();
    let files = repo.status().unwrap();
    let entry = file(&files, "file");
    assert_eq!((entry.index, entry.worktree), ('M', 'M'));
    assert!(
        repo.diff(entry, View::All)
            .unwrap()
            .summary
            .unwrap()
            .contains("No net change")
    );
    assert!(has(
        &repo.diff(entry, View::Staged).unwrap(),
        Kind::Add,
        "staged"
    ));
    let unstaged = repo.diff(entry, View::Unstaged).unwrap();
    assert!(has(&unstaged, Kind::Delete, "staged"));
    assert!(has(&unstaged, Kind::Add, "head"));
    fixture.git(&["rm", "-f", "file"]);
    fixture.write("file", "recreated\n");
    let files = repo.status().unwrap();
    assert_eq!(files.len(), 1);
    let all = repo.diff(&files[0], View::All).unwrap();
    assert!(has(&all, Kind::Delete, "head"));
    assert!(has(&all, Kind::Add, "recreated"));
    let unstaged = repo.diff(&files[0], View::Unstaged).unwrap();
    assert!(has(&unstaged, Kind::Add, "recreated"));
}

#[test]
fn unborn_repository_and_empty_untracked_files() {
    let fixture = Fixture::new();
    fixture.write("new", "index\n");
    fixture.git(&["add", "new"]);
    fixture.write("new", "working\n");
    fixture.write("empty", "");
    let repo = fixture.repo();
    let files = repo.status().unwrap();
    assert!(has(
        &repo.diff(file(&files, "new"), View::All).unwrap(),
        Kind::Add,
        "working"
    ));
    assert!(has(
        &repo.diff(file(&files, "new"), View::Staged).unwrap(),
        Kind::Add,
        "index"
    ));
    assert!(has(
        &repo.diff(file(&files, "new"), View::Unstaged).unwrap(),
        Kind::Delete,
        "index"
    ));
    assert_eq!(
        repo.diff(file(&files, "empty"), View::All)
            .unwrap()
            .summary
            .as_deref(),
        Some("Empty untracked file.")
    );
}

#[test]
fn paths_are_literal_and_nul_delimited_including_renames() {
    let fixture = Fixture::new();
    let source = "old\nname\t.txt";
    let target = "dir/new\nname\t.txt";
    fixture.write(source, "old\n");
    fixture.write("other.txt", "leave\n");
    fixture.commit();
    fs::create_dir(fixture.path().join("dir")).unwrap();
    fixture.git(&["mv", source, target]);
    for path in ["*.txt", ":(glob)*", "-option", "日本語.txt"] {
        fixture.write(path, "literal\n");
    }
    let repo = fixture.repo();
    let files = repo.status().unwrap();
    assert_eq!(
        file(&files, target).original.as_deref(),
        Some(Path::new(source))
    );
    for path in ["*.txt", ":(glob)*", "-option", "日本語.txt"] {
        assert!(has(
            &repo.diff(file(&files, path), View::All).unwrap(),
            Kind::Add,
            "literal"
        ));
    }
    fixture.git(&["add", "--", "*.txt"]);
    fixture.write("*.txt", "changed\n");
    let files = repo.status().unwrap();
    let diff = repo.diff(file(&files, "*.txt"), View::Unstaged).unwrap();
    assert!(has(&diff, Kind::Add, "changed"));
    assert!(!diff.rows.iter().any(|row| row.text == "leave"));
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_and_untracked_symlinks_are_readable() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, fs::symlink},
    };
    let fixture = Fixture::new();
    let path = PathBuf::from(OsString::from_vec(b"invalid-\xff".to_vec()));
    fixture.write(&path, "bytes\n");
    symlink("missing-target", fixture.path().join("link")).unwrap();
    let repo = fixture.repo();
    let files = repo.status().unwrap();
    let entry = files.iter().find(|entry| entry.path == path).unwrap();
    assert!(has(
        &repo.diff(entry, View::All).unwrap(),
        Kind::Add,
        "bytes"
    ));
    assert!(has(
        &repo.diff(file(&files, "link"), View::All).unwrap(),
        Kind::Add,
        "missing-target"
    ));
}

#[test]
fn subdirectory_and_linked_worktree_discovery() {
    let fixture = Fixture::new();
    fixture.write("nested/file", "base\n");
    fixture.commit();
    assert_eq!(
        Repository::discover(&fixture.path().join("nested"))
            .unwrap()
            .root,
        fixture.path()
    );
    let target = tempfile::tempdir().unwrap();
    let worktree = target.path().join("worktree");
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "linked",
        worktree.to_str().unwrap(),
    ]);
    fs::write(worktree.join("nested/file"), "linked\n").unwrap();
    let repo = Repository::discover(&worktree.join("nested")).unwrap();
    assert_eq!(repo.root, worktree);
    assert_eq!(repo.status().unwrap().len(), 1);
    assert!(fixture.repo().status().unwrap().is_empty());
}

#[test]
fn binary_files_and_conflicts_have_explicit_summaries() {
    let fixture = Fixture::new();
    fixture.write("binary", b"old\0bytes");
    fixture.write("conflict", "base\n");
    fixture.commit();
    fixture.git(&["checkout", "-qb", "branch"]);
    fixture.write("conflict", "branch\n");
    fixture.commit();
    fixture.git(&["checkout", "-q", "main"]);
    fixture.write("conflict", "main\n");
    fixture.commit();
    let status = Command::new("git")
        .arg("-C")
        .arg(fixture.path())
        .args(["merge", "branch"])
        .output()
        .unwrap();
    assert!(!status.status.success());
    fixture.write("binary", b"new\0bytes");
    fixture.write("untracked-binary", b"new\0bytes");
    let repo = fixture.repo();
    let files = repo.status().unwrap();
    for name in ["binary", "untracked-binary"] {
        assert!(
            repo.diff(file(&files, name), View::All)
                .unwrap()
                .summary
                .unwrap()
                .contains("Binary file")
        );
    }
    for view in [View::All, View::Staged, View::Unstaged] {
        assert!(
            repo.diff(file(&files, "conflict"), view)
                .unwrap()
                .summary
                .unwrap()
                .contains("Unmerged conflict")
        );
    }
}

#[test]
fn repeated_edits_refresh_without_moving_the_reader_and_commits_remove_entries() {
    let fixture = Fixture::new();
    fixture.write("a", "base\n");
    fixture.write("b", "base\n");
    fixture.commit();
    fixture.write("a", "first\n");
    fixture.write("b", "second\n");
    let repo = fixture.repo();
    let mut app = App::new(repo.root.clone());
    refresh_app(&repo, &mut app);
    refresh_app(&repo, &mut app);
    app.position.vertical = 2;
    app.position.horizontal = 4;
    for text in ["again", "third"] {
        fixture.write("a", format!("{text}\n"));
        refresh_app(&repo, &mut app);
        assert!(has(app.diff.as_ref().unwrap(), Kind::Add, text));
        assert_eq!(app.position.vertical, 2);
        assert_eq!(app.position.horizontal, 4);
        assert_eq!(app.selected.as_deref(), Some(Path::new("a")));
    }
    let request = app.request();
    let result = refresh(&repo, &request);
    app.select("b".into());
    assert!(!app.apply(Response { request, result }));
    assert!(app.diff.is_none());
    let request = app.request();
    let result = refresh(&repo, &request);
    app.set_view(View::Staged);
    assert!(!app.apply(Response { request, result }));
    app.set_view(View::All);
    fixture.commit();
    refresh_app(&repo, &mut app);
    assert!(app.files.is_empty());
    assert_eq!(app.selected.as_deref(), Some(Path::new("b")));
    assert!(
        app.diff
            .as_ref()
            .unwrap()
            .summary
            .as_ref()
            .unwrap()
            .contains("no changes")
    );
}

#[test]
fn worker_accepts_only_one_refresh_at_a_time() {
    let fixture = Fixture::new();
    fixture.write("file", "content\n");
    let repo = fixture.repo();
    let mut app = App::new(repo.root.clone());
    app.select("file".into());
    let mut worker = Worker::new(repo).unwrap();
    assert!(worker.request(app.request()).unwrap());
    assert!(!worker.request(app.request()).unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(response) = worker.receive().unwrap() {
            assert!(app.apply(response));
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!worker.busy());
    assert!(has(app.diff.as_ref().unwrap(), Kind::Add, "content"));
}

fn render_app() -> App {
    let mut app = App::new("/repository".into());
    app.files = vec![FileEntry {
        path: "src/main.rs".into(),
        original: None,
        index: ' ',
        worktree: 'M',
    }];
    app.select("src/main.rs".into());
    app.tree_cursor = app.selected.clone();
    app.diff = Some(Diff::parse(
        "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    let message = \"old café\";\n+    let message = \"new café\";\n }\n@@ -20 +20 @@\n-old tail\n+new tail\n",
    ));
    app
}

fn render(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn layouts_resize_and_render_change_and_intraline_colors() {
    let mut app = render_app();
    for (width, split) in [(160, true), (90, false)] {
        let terminal = render(&mut app, width, 24);
        assert_eq!(app.split, split);
        let text = screen(&terminal);
        assert!(text.contains("old café"));
        assert!(text.contains("new café"));
        let cells = &terminal.backend().buffer().content;
        for color in [
            ui::DELETED,
            ui::ADDED,
            ui::DELETED_EMPHASIS,
            ui::ADDED_EMPHASIS,
        ] {
            assert!(
                cells.iter().any(|cell| cell.bg == color),
                "missing color {color:?}"
            );
        }
    }
    app.focus = Focus::Diff;
    key(&mut app, KeyCode::Char(']'));
    assert!(app.position.vertical > 0);
    let anchor = app
        .diff
        .as_ref()
        .unwrap()
        .row_indices(app.position.vertical, app.split)[0];
    render(&mut app, 160, 24);
    assert!(
        app.diff
            .as_ref()
            .unwrap()
            .row_indices(app.position.vertical, app.split)
            .contains(&anchor)
    );
    key(&mut app, KeyCode::Char('t'));
    render(&mut app, 90, 24);
    assert_eq!(app.tree_area.width, 0);
    app.layout = LayoutMode::Split;
    for (width, height) in [(1, 1), (2, 2), (10, 5), (50, 12)] {
        render(&mut app, width, height);
        app.help = true;
        render(&mut app, width, height);
        app.help = false;
    }
}

#[test]
fn navigation_search_capture_and_mouse_scrolling() {
    let mut app = render_app();
    render(&mut app, 100, 24);
    key(&mut app, KeyCode::Left);
    assert_eq!(app.tree_cursor.as_deref(), Some(Path::new("src")));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.tree_rows().len(), 1);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.tree_rows().len(), 2);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Diff);
    key(&mut app, KeyCode::Char('/'));
    for c in "qvtr?[]".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert!(!app.quit && !app.help);
    assert_eq!(app.layout, LayoutMode::Auto);
    assert_eq!(app.search_input.as_deref(), Some("qvtr?[]"));
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('/'));
    for c in "new tail".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.position.vertical > 0);
    key(&mut app, KeyCode::Home);
    app.mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: app.diff_area.x,
        row: app.diff_area.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.position.vertical, 3);
}

#[test]
fn patch_parser_keeps_header_like_content_and_line_numbers() {
    let diff = Diff::parse(
        "--- old\n+++ new\n@@ -7,2 +9,2 @@\n--- old-looking content\n+++ new-looking content\n context\n\\ No newline at end of file\n",
    );
    assert!(has(&diff, Kind::Delete, "-- old-looking content"));
    assert!(has(&diff, Kind::Add, "++ new-looking content"));
    let context = diff
        .rows
        .iter()
        .find(|row| row.kind == Kind::Context)
        .unwrap();
    assert_eq!((context.old, context.new), (Some(8), Some(10)));
    assert!(diff.rows.iter().any(|row| row.text.contains("No newline")));
}

#[test]
fn git_display_configuration_and_missing_final_newlines_preserve_diff_structure() {
    let fixture = Fixture::new();
    fixture.write("file", "old text\n\ntail");
    fixture.commit();
    fixture.write("file", "new text\n\ntail changed");
    fixture.git(&["config", "diff.outputIndicatorNew", ">"]);
    fixture.git(&["config", "diff.outputIndicatorOld", "<"]);
    fixture.git(&["config", "diff.outputIndicatorContext", "="]);
    fixture.git(&["config", "diff.suppressBlankEmpty", "true"]);
    let repo = fixture.repo();
    let files = repo.status().unwrap();
    let diff = repo.diff(&files[0], View::All).unwrap();
    assert!(has(&diff, Kind::Context, ""));
    assert!(has(&diff, Kind::Add, "new text"));
    assert!(
        diff.rows
            .iter()
            .any(|row| row.text == "tail changed" && !row.emphasis.is_empty())
    );
    assert!(diff.pairs.iter().any(|pair| {
        pair.left.is_some_and(|i| diff.rows[i].text == "tail")
            && pair
                .right
                .is_some_and(|i| diff.rows[i].text == "tail changed")
    }));
}
