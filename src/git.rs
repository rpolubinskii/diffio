use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use crate::diff::Diff;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    #[default]
    All,
    Staged,
    Unstaged,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Staged => "Staged",
            Self::Unstaged => "Unstaged",
        }
    }

    pub fn comparison(self) -> &'static str {
        match self {
            Self::All => "HEAD → working tree",
            Self::Staged => "HEAD → index",
            Self::Unstaged => "index → working tree",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub original: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
}

impl FileEntry {
    pub fn untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }

    pub fn conflicted(&self) -> bool {
        self.index == 'U'
            || self.worktree == 'U'
            || matches!((self.index, self.worktree), ('A', 'A') | ('D', 'D'))
    }

    pub fn visible(&self, view: View) -> bool {
        self.conflicted()
            || match view {
                View::All => true,
                View::Staged => !matches!(self.index, ' ' | '?'),
                View::Unstaged => self.worktree != ' ',
            }
    }

    pub fn marker(&self, view: View) -> char {
        if self.conflicted() {
            'U'
        } else if self.untracked() {
            '?'
        } else {
            match view {
                View::Staged => self.index,
                View::Unstaged => self.worktree,
                View::All if self.index != ' ' => self.index,
                View::All => self.worktree,
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
}

impl Repository {
    pub fn discover(path: &Path) -> Result<Self> {
        let output = command(path)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|e| format!("Cannot run Git in {}: {e}", path.display()))?;
        ensure_success(&output, "discover repository")?;
        let bytes = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
        Ok(Self {
            root: bytes_path(bytes)?,
        })
    }

    pub fn status(&self) -> Result<Vec<FileEntry>> {
        let output = self.output(&[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--renames",
            "--ignore-submodules=none",
        ])?;
        ensure_success(&output, "read repository status")?;
        parse_status(&output.stdout)
    }

    pub fn diff(&self, entry: &FileEntry, view: View) -> Result<Diff> {
        if entry.conflicted() {
            return Ok(Diff::message(format!(
                "Unmerged conflict ({}{}). Git has multiple index stages for this path.\nResolve the conflict in your editor; this viewer is read-only.",
                entry.index, entry.worktree
            )));
        }
        let has_head = self
            .output(&["rev-parse", "--verify", "--quiet", "HEAD"])?
            .status
            .success();
        if entry.untracked()
            || (view == View::Unstaged && entry.worktree == '?')
            || (view == View::All && (!has_head || entry.index == 'D'))
        {
            return self.working_content_diff(entry, has_head && view == View::All);
        }

        let mut cmd = self.diff_command();
        if view == View::Staged {
            cmd.arg("--cached");
        } else if view == View::All {
            cmd.arg("HEAD");
        }
        cmd.arg("--").arg(&entry.path);
        if let Some(original) = &entry.original {
            cmd.arg(original);
        }
        let output = cmd.output().map_err(|e| format!("Cannot read diff: {e}"))?;
        ensure_success(&output, &format!("read diff for {}", entry.path.display()))?;
        Ok(parse_diff(&output.stdout, entry, view))
    }

    fn working_content_diff(&self, entry: &FileEntry, compare_head: bool) -> Result<Diff> {
        let absolute = self.root.join(&entry.path);
        if absolute.is_dir() {
            return Ok(Diff::message(
                "Directory or submodule changed; no text diff available.",
            ));
        }
        if fs::symlink_metadata(&absolute).is_ok_and(|m| m.len() > 8 * 1024 * 1024) {
            return Ok(Diff::message("File exceeds the 8 MiB preview limit."));
        }
        let old = if compare_head {
            let mut object = OsString::from("HEAD:");
            object.push(&entry.path);
            let output = command(&self.root)
                .arg("show")
                .arg(object)
                .output()
                .map_err(|e| format!("Cannot read HEAD content: {e}"))?;
            // A staged deletion may have been recreated as an untracked file.
            if output.status.success() {
                output.stdout
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let new = match fs::symlink_metadata(&absolute) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&absolute).map_err(|e| e.to_string())?;
                target.as_os_str().as_encoded_bytes().to_vec()
            }
            Ok(_) => fs::read(&absolute)
                .map_err(|e| format!("Cannot read {}: {e}", absolute.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("Cannot inspect {}: {e}", absolute.display())),
        };
        let temp = tempfile::tempdir().map_err(|e| format!("Cannot create diff snapshot: {e}"))?;
        let before = temp.path().join("before");
        let after = temp.path().join("after");
        fs::write(&before, old)
            .and_then(|()| fs::write(&after, new))
            .map_err(|e| format!("Cannot write temporary diff snapshot: {e}"))?;
        let output = self
            .diff_command()
            .arg("--no-index")
            .arg("--")
            .arg(before)
            .arg(after)
            .output()
            .map_err(|e| format!("Cannot compare working content: {e}"))?;
        if !matches!(output.status.code(), Some(0 | 1)) {
            ensure_success(&output, "compare working content")?;
        }
        let mut diff = parse_diff(&output.stdout, entry, View::All);
        if output.stdout.is_empty() && entry.untracked() {
            diff = Diff::message("Empty untracked file.");
        }
        Ok(diff)
    }

    fn diff_command(&self) -> Command {
        let mut cmd = command(&self.root);
        cmd.args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--word-diff=none",
            "--no-relative",
            "--no-prefix",
            "--output-indicator-new=+",
            "--output-indicator-old=-",
            "--output-indicator-context= ",
            "--unified=3",
            "--find-renames",
            "--ignore-submodules=none",
            "--submodule=short",
        ]);
        cmd
    }

    fn output(&self, args: &[&str]) -> Result<Output> {
        command(&self.root)
            .args(args)
            .output()
            .map_err(|e| format!("Git {} failed in {}: {e}", args[0], self.root.display()))
    }
}

fn command(path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("--no-pager")
        .arg("--literal-pathspecs")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "color.ui=false",
            "-c",
            "diff.suppressBlankEmpty=false",
        ])
        .arg("-C")
        .arg(path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null());
    cmd
}

fn ensure_success(output: &Output, operation: &str) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Git could not {operation}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn parse_diff(bytes: &[u8], entry: &FileEntry, view: View) -> Diff {
    if bytes.len() > 8 * 1024 * 1024 {
        return Diff::message("Patch exceeds the 8 MiB preview limit.");
    }
    if bytes.is_empty() {
        return Diff::message(if view == View::All && entry.index != ' ' {
            "No net change from HEAD. Staged changes remain; press 2 to inspect the index."
        } else {
            "No text changes in this view."
        });
    }
    Diff::parse(&String::from_utf8_lossy(bytes))
}

fn parse_status(bytes: &[u8]) -> Result<Vec<FileEntry>> {
    let mut records = bytes.split(|b| *b == 0).filter(|record| !record.is_empty());
    let mut entries: BTreeMap<PathBuf, FileEntry> = BTreeMap::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err("Git returned malformed porcelain status".into());
        }
        let index = record[0] as char;
        let worktree = record[1] as char;
        let path = bytes_path(&record[3..])?;
        let original = if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            Some(bytes_path(
                records
                    .next()
                    .ok_or("Missing rename source in Git status")?,
            )?)
        } else {
            None
        };
        if let Some(existing) = entries.get_mut(&path) {
            existing.worktree = worktree;
        } else {
            entries.insert(
                path.clone(),
                FileEntry {
                    path,
                    original,
                    index,
                    worktree,
                },
            );
        }
    }
    Ok(entries.into_values().collect())
}

#[cfg(unix)]
fn bytes_path(bytes: &[u8]) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn bytes_path(bytes: &[u8]) -> Result<PathBuf> {
    std::str::from_utf8(bytes)
        .map(PathBuf::from)
        .map_err(|e| format!("Invalid Git path: {e}"))
}

pub fn display_path(path: &Path) -> String {
    display_text(&path.to_string_lossy())
}

pub fn display_text(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '\t' => "    ".chars().collect::<Vec<_>>(),
            c if c.is_control() => c.escape_default().collect(),
            c => vec![c],
        })
        .collect()
}
