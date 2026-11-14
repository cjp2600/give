//! Bounded, shared previews. Slots coalesce foreground and speculative reads.
//! `ready` and `styles` never wait; only background workers call `load`/`highlight`.
use crate::{
    git::{ChangedFile, Diff, LineKind, Repository},
    preview::{self, Highlight},
};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

pub type Syntax = Arc<Vec<Arc<OnceLock<Vec<Highlight>>>>>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: PathBuf,
    pub version: u64,
}

#[derive(Clone)]
pub struct Pair {
    pub normal: Arc<Diff>,
    pub deletions: Arc<Diff>,
}

impl Pair {
    pub fn mode(&self, deletions: bool) -> Arc<Diff> {
        if deletions {
            self.deletions.clone()
        } else {
            self.normal.clone()
        }
    }
}

pub struct Slot {
    pub key: Key,
    content: OnceLock<Result<Pair, String>>,
    syntax: OnceLock<(Syntax, Syntax)>,
    normal_parser: Mutex<preview::HighlightCache>,
    old_parser: Mutex<preview::HighlightCache>,
    weight: AtomicUsize,
}

impl Slot {
    pub fn ready(&self) -> Option<&Result<Pair, String>> {
        self.content.get()
    }

    pub fn styles(&self, deletions: bool) -> Option<Syntax> {
        self.syntax.get().map(|(normal, deleted)| {
            if deletions {
                deleted.clone()
            } else {
                normal.clone()
            }
        })
    }

    pub fn load(&self, repo: &Repository, file: &ChangedFile) -> Result<Pair, String> {
        self.load_with(|| repo.preview_pair(file))
    }

    /// Loads from another source, such as a branch review, with the same sharing.
    pub fn load_with(
        &self,
        read: impl FnOnce() -> anyhow::Result<(Diff, Diff)>,
    ) -> Result<Pair, String> {
        let result = self
            .content
            .get_or_init(|| {
                read()
                    .map(|(normal, deletions)| {
                        let bytes = [&normal, &deletions]
                            .into_iter()
                            .map(|diff| {
                                diff.raw.len()
                                    + diff
                                        .lines
                                        .iter()
                                        .map(|line| line.text.len() + std::mem::size_of_val(line))
                                        .sum::<usize>()
                            })
                            .sum();
                        self.weight.store(bytes, Ordering::Relaxed);
                        Pair {
                            normal: Arc::new(normal),
                            deletions: Arc::new(deletions),
                        }
                    })
                    .map_err(|error| format!("{error:#}"))
            })
            .clone();
        if result.is_ok() {
            self.init_syntax();
        }
        result
    }

    fn init_syntax(&self) {
        let Some(Ok(pair)) = self.ready() else {
            return;
        };
        self.syntax.get_or_init(|| {
            let normal: Vec<_> = pair
                .normal
                .lines
                .iter()
                .map(|_| Arc::new(OnceLock::new()))
                .collect();
            let by_line: HashMap<_, _> = pair
                .normal
                .lines
                .iter()
                .enumerate()
                .filter_map(|(i, line)| line.new.map(|number| (number, i)))
                .collect();
            let deletions = pair
                .deletions
                .lines
                .iter()
                .map(|line| {
                    line.new
                        .and_then(|number| by_line.get(&number))
                        .map(|i| normal[*i].clone())
                        .unwrap_or_else(|| Arc::new(OnceLock::new()))
                })
                .collect();
            (Arc::new(normal), Arc::new(deletions))
        });
    }

    pub fn range_ready(&self, deletions: bool, range: Range<usize>) -> bool {
        self.styles(deletions).is_some_and(|syntax| {
            syntax
                .iter()
                .take(range.end)
                .skip(range.start)
                .all(|row| row.get().is_some())
        })
    }

    /// Retain spans only for visible chunks. Parser checkpoints preserve context
    /// across chunks; deleted lines use the old stream without affecting new text.
    pub fn highlight_range(&self, deletions: bool, range: Range<usize>) {
        self.init_syntax();
        let (Some(Ok(pair)), Some(syntax)) = (self.ready(), self.styles(deletions)) else {
            return;
        };
        let diff = pair.mode(deletions);
        let end = range.end.min(diff.lines.len());
        if range.start >= end || self.range_ready(deletions, range.clone()) {
            return;
        }
        if deletions {
            // Shared cells always get colors from the current file, never from
            // removed text that might open a comment, string, or embedded block.
            let numbers: std::collections::HashSet<_> = diff.lines[range.start..end]
                .iter()
                .filter_map(|line| line.new)
                .collect();
            let indices: Vec<_> = pair
                .normal
                .lines
                .iter()
                .enumerate()
                .filter_map(|(i, line)| line.new.filter(|n| numbers.contains(n)).map(|_| i))
                .collect();
            if let (Some(&start), Some(&last)) = (indices.first(), indices.last()) {
                self.highlight_range(false, start..last + 1);
            }
            if self.range_ready(true, range.start..end) {
                return;
            }
        }
        let parser = if deletions {
            &self.old_parser
        } else {
            &self.normal_parser
        };
        let spans = parser.lock().unwrap().range(
            &self.key.path,
            &diff.lines,
            range.start..end,
            Some(if deletions {
                LineKind::Added
            } else {
                LineKind::Removed
            }),
        );
        for (index, spans) in (range.start..end).zip(spans) {
            syntax[index].get_or_init(|| {
                self.weight.fetch_add(
                    spans.len() * std::mem::size_of::<Highlight>(),
                    Ordering::Relaxed,
                );
                spans
            });
        }
    }

    /// Full warm-up for explicit benchmarks; the UI requests visible chunks instead.
    pub fn highlight(&self) {
        self.highlight_range(false, 0..usize::MAX);
        self.highlight_range(true, 0..usize::MAX);
    }

    fn bytes(&self) -> usize {
        self.weight.load(Ordering::Relaxed)
    }
}

pub struct PreviewCache {
    slots: VecDeque<Arc<Slot>>,
    capacity: usize,
    byte_limit: usize,
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self::new(12, 24 * 1024 * 1024)
    }
}

impl PreviewCache {
    pub fn new(capacity: usize, byte_limit: usize) -> Self {
        Self {
            slots: VecDeque::new(),
            capacity: capacity.max(1),
            byte_limit,
        }
    }

    /// At most 12 comparisons and Arc increments on the UI thread.
    pub fn slot(&mut self, key: Key) -> Arc<Slot> {
        if let Some(index) = self.slots.iter().position(|slot| slot.key == key) {
            let slot = self.slots.remove(index).unwrap();
            self.slots.push_front(slot.clone());
            return slot;
        }
        self.slots.retain(|slot| slot.key.path != key.path);
        let slot = Arc::new(Slot {
            key,
            content: OnceLock::new(),
            syntax: OnceLock::new(),
            normal_parser: Mutex::default(),
            old_parser: Mutex::default(),
            weight: AtomicUsize::new(0),
        });
        self.slots.push_front(slot.clone());
        self.trim();
        slot
    }

    pub fn retain_versions(&mut self, versions: &HashMap<PathBuf, u64>) {
        self.slots
            .retain(|slot| versions.get(&slot.key.path) == Some(&slot.key.version));
    }

    pub fn remove(&mut self, path: &Path) {
        self.slots.retain(|slot| slot.key.path != path);
    }

    pub fn trim(&mut self) {
        // Keep the most recently requested slot even if one file exceeds the budget.
        while self.slots.len() > 1
            && (self.slots.len() > self.capacity || self.bytes() > self.byte_limit)
        {
            self.slots.pop_back();
        }
    }

    pub fn bytes(&self) -> usize {
        self.slots.iter().map(|slot| slot.bytes()).sum()
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
