use std::{ops::Range, time::Duration};

use similar::{ChangeTag, TextDiff};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hunk,
    Context,
    Delete,
    Add,
    Meta,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub kind: Kind,
    pub text: String,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub emphasis: Vec<Range<usize>>,
}

impl Row {
    fn new(kind: Kind, text: &str, old: Option<usize>, new: Option<usize>) -> Self {
        Self {
            kind,
            text: text.into(),
            old,
            new,
            emphasis: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Pair {
    pub left: Option<usize>,
    pub right: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Diff {
    pub rows: Vec<Row>,
    pub pairs: Vec<Pair>,
    pub summary: Option<String>,
}

impl Diff {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            summary: Some(message.into()),
            ..Self::default()
        }
    }

    pub fn parse(patch: &str) -> Self {
        let mut diff = Self::default();
        let mut old = 0;
        let mut new = 0;
        let mut remaining: (usize, usize) = (0, 0);
        for line in patch.lines() {
            if remaining != (0, 0) {
                let (kind, old_number, new_number) = match line.as_bytes().first() {
                    Some(b' ') => (Kind::Context, Some(old), Some(new)),
                    Some(b'-') => (Kind::Delete, Some(old), None),
                    Some(b'+') => (Kind::Add, None, Some(new)),
                    _ => {
                        diff.rows.push(Row::new(Kind::Meta, line, None, None));
                        continue;
                    }
                };
                diff.rows
                    .push(Row::new(kind, &line[1..], old_number, new_number));
                if old_number.is_some() {
                    old += 1;
                    remaining.0 = remaining.0.saturating_sub(1);
                }
                if new_number.is_some() {
                    new += 1;
                    remaining.1 = remaining.1.saturating_sub(1);
                }
            } else if line.starts_with("@@ ") {
                if let Some((old_start, old_count, new_start, new_count)) = hunk_range(line) {
                    old = old_start;
                    new = new_start;
                    remaining = (old_count, new_count);
                    diff.rows.push(Row::new(Kind::Hunk, line, None, None));
                }
            } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
                return Self::message("Binary file changed; no text diff available.");
            } else if !line.starts_with("diff --git ")
                && !line.starts_with("index ")
                && !line.starts_with("--- ")
                && !line.starts_with("+++ ")
            {
                diff.rows.push(Row::new(Kind::Meta, line, None, None));
            }
        }
        diff.align();
        if diff.rows.is_empty() {
            diff.summary = Some("No text changes in this view.".into());
        }
        diff
    }

    fn align(&mut self) {
        let mut index = 0;
        while index < self.rows.len() {
            if matches!(self.rows[index].kind, Kind::Delete | Kind::Add) {
                let start = index;
                while index < self.rows.len()
                    && (matches!(self.rows[index].kind, Kind::Delete | Kind::Add)
                        || self.rows[index].text == "\\ No newline at end of file")
                {
                    index += 1;
                }
                let removed: Vec<_> = (start..index)
                    .filter(|&i| self.rows[i].kind == Kind::Delete)
                    .collect();
                let added: Vec<_> = (start..index)
                    .filter(|&i| self.rows[i].kind == Kind::Add)
                    .collect();
                for pair_index in 0..removed.len().max(added.len()) {
                    let left = removed.get(pair_index).copied();
                    let right = added.get(pair_index).copied();
                    if let (Some(left), Some(right)) = (left, right) {
                        let (before, after) =
                            changed_ranges(&self.rows[left].text, &self.rows[right].text);
                        self.rows[left].emphasis = before;
                        self.rows[right].emphasis = after;
                    }
                    self.pairs.push(Pair { left, right });
                }
                for note in (start..index).filter(|&i| self.rows[i].kind == Kind::Meta) {
                    self.pairs.push(Pair {
                        left: Some(note),
                        right: Some(note),
                    });
                }
            } else {
                self.pairs.push(Pair {
                    left: Some(index),
                    right: Some(index),
                });
                index += 1;
            }
        }
    }

    pub fn len(&self, split: bool) -> usize {
        if split {
            self.pairs.len()
        } else {
            self.rows.len()
        }
    }

    pub fn row_indices(&self, position: usize, split: bool) -> Vec<usize> {
        if split {
            self.pairs
                .get(position)
                .map(|pair| {
                    let mut indices: Vec<_> = pair.left.into_iter().chain(pair.right).collect();
                    indices.dedup();
                    indices
                })
                .unwrap_or_default()
        } else if position < self.rows.len() {
            vec![position]
        } else {
            Vec::new()
        }
    }

    pub fn is_hunk(&self, position: usize, split: bool) -> bool {
        self.row_indices(position, split)
            .iter()
            .any(|&i| self.rows[i].kind == Kind::Hunk)
    }
}

fn hunk_range(header: &str) -> Option<(usize, usize, usize, usize)> {
    let mut fields = header.split_whitespace().skip(1);
    let (old, old_count) = range(fields.next()?.strip_prefix('-')?)?;
    let (new, new_count) = range(fields.next()?.strip_prefix('+')?)?;
    Some((old, old_count, new, new_count))
}

fn range(text: &str) -> Option<(usize, usize)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

fn changed_ranges(before: &str, after: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let diff = TextDiff::configure()
        .timeout(Duration::from_millis(5))
        .diff_chars(before, after);
    let mut old_offset = 0;
    let mut new_offset = 0;
    let mut old = Vec::new();
    let mut new = Vec::new();
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                old_offset += len;
                new_offset += len;
            }
            ChangeTag::Delete => {
                extend_range(&mut old, old_offset..old_offset + len);
                old_offset += len;
            }
            ChangeTag::Insert => {
                extend_range(&mut new, new_offset..new_offset + len);
                new_offset += len;
            }
        }
    }
    (old, new)
}

fn extend_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    if let Some(last) = ranges.last_mut()
        && last.end == range.start
    {
        last.end = range.end;
    } else {
        ranges.push(range);
    }
}
