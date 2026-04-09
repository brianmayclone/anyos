//! Simple line-based diff for git status and git diff.

use alloc::string::String;
use alloc::vec::Vec;

/// A diff hunk.
#[derive(Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

/// A single line in a diff.
#[derive(Debug)]
pub enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
}

/// Compute a unified diff between two texts.
pub fn unified_diff(old: &str, new: &str, context: usize) -> Vec<DiffHunk> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Simple LCS-based diff using Myers' algorithm (simplified)
    let edits = compute_edits(&old_lines, &new_lines);
    group_into_hunks(&edits, &old_lines, &new_lines, context)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Edit {
    Equal,
    Insert,
    Delete,
}

fn compute_edits(old: &[&str], new: &[&str]) -> Vec<Edit> {
    let n = old.len();
    let m = new.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }

    // Simple O(NM) DP for LCS
    let mut dp = alloc::vec![alloc::vec![0u32; m + 1]; n + 1];

    for i in 1..=n {
        for j in 1..=m {
            if old[i - 1] == new[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = core::cmp::max(dp[i - 1][j], dp[i][j - 1]);
            }
        }
    }

    // Backtrack to get edit sequence
    let mut edits = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            edits.push(Edit::Equal);
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            edits.push(Edit::Insert);
            j -= 1;
        } else {
            edits.push(Edit::Delete);
            i -= 1;
        }
    }

    edits.reverse();
    edits
}

fn group_into_hunks(
    edits: &[Edit],
    old_lines: &[&str],
    new_lines: &[&str],
    context: usize,
) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut old_idx = 0usize;
    let mut new_idx = 0usize;
    let mut i = 0;

    while i < edits.len() {
        // Skip equal lines
        if edits[i] == Edit::Equal {
            old_idx += 1;
            new_idx += 1;
            i += 1;
            continue;
        }

        // Found a change — create a hunk with context
        let hunk_start = i;
        let ctx_start_old = old_idx.saturating_sub(context);
        let ctx_start_new = new_idx.saturating_sub(context);

        let mut lines = Vec::new();

        // Add leading context
        let ctx_before = old_idx - ctx_start_old;
        for k in 0..ctx_before {
            if ctx_start_old + k < old_lines.len() {
                lines.push(DiffLine::Context(String::from(old_lines[ctx_start_old + k])));
            }
        }

        // Add changes and interleaved context
        while i < edits.len() {
            match edits[i] {
                Edit::Equal => {
                    // Check if there's another change within context distance
                    let mut next_change = false;
                    for k in 1..=(context * 2 + 1) {
                        if i + k < edits.len() && edits[i + k] != Edit::Equal {
                            next_change = true;
                            break;
                        }
                    }
                    if next_change {
                        lines.push(DiffLine::Context(String::from(old_lines[old_idx])));
                        old_idx += 1;
                        new_idx += 1;
                    } else {
                        break;
                    }
                }
                Edit::Delete => {
                    lines.push(DiffLine::Removed(String::from(old_lines[old_idx])));
                    old_idx += 1;
                }
                Edit::Insert => {
                    lines.push(DiffLine::Added(String::from(new_lines[new_idx])));
                    new_idx += 1;
                }
            }
            i += 1;
        }

        // Add trailing context
        let mut trailing = 0;
        while trailing < context && i < edits.len() && edits[i] == Edit::Equal {
            lines.push(DiffLine::Context(String::from(old_lines[old_idx])));
            old_idx += 1;
            new_idx += 1;
            trailing += 1;
            i += 1;
        }

        let old_count = lines.iter().filter(|l| !matches!(l, DiffLine::Added(_))).count();
        let new_count = lines.iter().filter(|l| !matches!(l, DiffLine::Removed(_))).count();

        hunks.push(DiffHunk {
            old_start: ctx_start_old + 1,
            old_count,
            new_start: ctx_start_new + 1,
            new_count,
            lines,
        });
    }

    hunks
}

/// Format a diff in unified diff format.
pub fn format_diff(path: &str, hunks: &[DiffHunk]) -> String {
    let mut out = String::new();
    out.push_str(&alloc::format!("--- a/{}\n", path));
    out.push_str(&alloc::format!("+++ b/{}\n", path));

    for hunk in hunks {
        out.push_str(&alloc::format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_count,
            hunk.new_start, hunk.new_count,
        ));
        for line in &hunk.lines {
            match line {
                DiffLine::Context(s) => {
                    out.push(' ');
                    out.push_str(s);
                    out.push('\n');
                }
                DiffLine::Added(s) => {
                    out.push('+');
                    out.push_str(s);
                    out.push('\n');
                }
                DiffLine::Removed(s) => {
                    out.push('-');
                    out.push_str(s);
                    out.push('\n');
                }
            }
        }
    }
    out
}
