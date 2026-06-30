use std::env;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionKind {
    Command,
    Directory,
    File,
    Variable,
    Alias,
    Flag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub replacement: String,
    pub range: Range<usize>,
    pub kind: CompletionKind,
    pub description: String,
    pub score: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionBatch {
    pub revision: u64,
    pub items: Vec<CompletionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    pub input: String,
    pub cursor: usize,
    pub cwd: PathBuf,
    pub path_dirs: Vec<PathBuf>,
    pub aliases: Vec<(String, String)>,
    pub revision: u64,
}

impl CompletionRequest {
    pub fn from_env(input: String, cursor: usize, cwd: PathBuf, revision: u64) -> Self {
        let path_dirs = env::var_os("PATH")
            .map(|value| env::split_paths(&value).collect())
            .unwrap_or_default();
        Self {
            input,
            cursor,
            cwd,
            path_dirs,
            aliases: Vec::new(),
            revision,
        }
    }
}

pub fn complete(request: &CompletionRequest) -> Vec<CompletionItem> {
    let context = cursor_context(&request.input, request.cursor);
    let mut items = Vec::new();
    if context.prefix.starts_with('$') || context.raw_prefix.starts_with('$') {
        complete_variables(&context, &mut items);
    } else if context.command_position {
        complete_aliases(request, &context, &mut items);
        complete_commands(request, &context, &mut items);
        complete_paths(request, &context, &mut items, true);
    } else if context.prefix.starts_with('-') {
        complete_flags(&context, &mut items);
    } else {
        complete_paths(request, &context, &mut items, false);
    }
    items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.label.cmp(&b.label)));
    items.dedup_by(|a, b| a.replacement == b.replacement && a.range == b.range);
    items.truncate(80);
    items
}

pub fn complete_if_current(
    request: &CompletionRequest,
    latest_revision: u64,
) -> Option<CompletionBatch> {
    if request.revision != latest_revision {
        return None;
    }
    Some(CompletionBatch {
        revision: request.revision,
        items: complete(request),
    })
}

pub fn apply_completion(input: &str, item: &CompletionItem) -> String {
    let mut out = String::new();
    out.push_str(&input[..item.range.start]);
    out.push_str(&item.replacement);
    out.push_str(&input[item.range.end..]);
    out
}

#[derive(Debug, Clone)]
struct CursorContext {
    prefix: String,
    raw_prefix: String,
    range: Range<usize>,
    command_position: bool,
    quote: QuoteMode,
    variable_braced: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteMode {
    None,
    Single,
    Double,
}

fn cursor_context(input: &str, cursor: usize) -> CursorContext {
    let cursor = cursor.min(input.len());
    let before = &input[..cursor];

    let mut token_start = 0;
    let mut token_has_content = false;
    let mut token_count_before_cursor = 0;
    let mut unescaped = String::new();
    let mut quote = QuoteMode::None;
    let mut escaped = false;

    for (index, ch) in before.char_indices() {
        if escaped {
            unescaped.push(ch);
            token_has_content = true;
            escaped = false;
            continue;
        }

        match quote {
            QuoteMode::None => match ch {
                '\\' => {
                    escaped = true;
                    token_has_content = true;
                }
                '\'' => {
                    quote = QuoteMode::Single;
                    if !token_has_content {
                        token_start = index + ch.len_utf8();
                    }
                    token_has_content = true;
                }
                '"' => {
                    quote = QuoteMode::Double;
                    if !token_has_content {
                        token_start = index + ch.len_utf8();
                    }
                    token_has_content = true;
                }
                ch if ch.is_whitespace() => {
                    if token_has_content {
                        token_count_before_cursor += 1;
                    }
                    token_start = index + ch.len_utf8();
                    token_has_content = false;
                    unescaped.clear();
                }
                _ => {
                    unescaped.push(ch);
                    token_has_content = true;
                }
            },
            QuoteMode::Single => {
                if ch == '\'' {
                    quote = QuoteMode::None;
                } else {
                    unescaped.push(ch);
                }
                token_has_content = true;
            }
            QuoteMode::Double => match ch {
                '"' => quote = QuoteMode::None,
                '\\' => escaped = true,
                _ => unescaped.push(ch),
            },
        }
    }

    let raw_prefix = input[token_start..cursor].to_owned();
    let (variable_braced, prefix) = variable_prefix(&unescaped);
    CursorContext {
        prefix,
        raw_prefix,
        range: token_start..cursor,
        command_position: token_count_before_cursor == 0,
        quote,
        variable_braced,
    }
}

fn variable_prefix(token: &str) -> (bool, String) {
    if let Some(prefix) = token.strip_prefix("${") {
        return (true, prefix.to_owned());
    }
    (false, token.to_owned())
}

fn complete_variables(context: &CursorContext, items: &mut Vec<CompletionItem>) {
    let prefix = context
        .prefix
        .trim_start_matches('$')
        .trim_end_matches('}')
        .to_ascii_uppercase();
    for (key, _) in env::vars() {
        if prefix_match(&key, &prefix) {
            let replacement = if context.variable_braced {
                format!("${{{key}}}")
            } else {
                format!("${key}")
            };
            items.push(CompletionItem {
                label: format!("${key}"),
                replacement,
                range: context.range.clone(),
                kind: CompletionKind::Variable,
                description: "environment variable".to_owned(),
                score: score(&key, &prefix),
            });
        }
    }
}

fn complete_aliases(
    request: &CompletionRequest,
    context: &CursorContext,
    items: &mut Vec<CompletionItem>,
) {
    for (name, expansion) in &request.aliases {
        if prefix_match(name, &context.prefix) {
            items.push(CompletionItem {
                label: name.clone(),
                replacement: format!("{} ", escape_for_context(name, context.quote, false)),
                range: context.range.clone(),
                kind: CompletionKind::Alias,
                description: expansion.clone(),
                score: score(name, &context.prefix) + 25,
            });
        }
    }
}

fn complete_commands(
    request: &CompletionRequest,
    context: &CursorContext,
    items: &mut Vec<CompletionItem>,
) {
    for dir in &request.path_dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten().take(512) {
                let name = entry.file_name().to_string_lossy().to_string();
                if prefix_match(&name, &context.prefix) {
                    items.push(CompletionItem {
                        label: name.clone(),
                        replacement: format!(
                            "{} ",
                            escape_for_context(&name, context.quote, false)
                        ),
                        range: context.range.clone(),
                        kind: CompletionKind::Command,
                        description: dir.display().to_string(),
                        score: score(&name, &context.prefix) + 20,
                    });
                }
            }
        }
    }
}

fn complete_paths(
    request: &CompletionRequest,
    context: &CursorContext,
    items: &mut Vec<CompletionItem>,
    command_position: bool,
) {
    let (base, prefix) = split_path_prefix(&request.cwd, &context.prefix);
    if let Ok(entries) = fs::read_dir(&base) {
        for entry in entries.flatten().take(512) {
            let name = entry.file_name().to_string_lossy().to_string();
            if !prefix.starts_with('.') && name.starts_with('.') {
                continue;
            }
            if !prefix_match(&name, &prefix) {
                continue;
            }
            let is_dir = entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false);
            let executable = !is_dir && is_executable(&entry.path());
            let suffix = if is_dir {
                std::path::MAIN_SEPARATOR.to_string()
            } else if command_position && executable {
                " ".to_owned()
            } else {
                String::new()
            };
            let replacement = if context.prefix.contains(std::path::MAIN_SEPARATOR) {
                let parent = Path::new(&context.prefix)
                    .parent()
                    .and_then(Path::to_str)
                    .unwrap_or("");
                let sep = if parent.is_empty() {
                    ""
                } else {
                    std::path::MAIN_SEPARATOR_STR
                };
                format!(
                    "{parent}{sep}{}{}",
                    escape_for_context(&name, context.quote, true),
                    suffix
                )
            } else {
                format!(
                    "{}{}",
                    escape_for_context(&name, context.quote, true),
                    suffix
                )
            };
            items.push(CompletionItem {
                label: name.clone(),
                replacement,
                range: context.range.clone(),
                kind: if is_dir {
                    CompletionKind::Directory
                } else {
                    CompletionKind::File
                },
                description: base.display().to_string(),
                score: score(&name, &prefix) + if is_dir { 10 } else { 0 },
            });
        }
    }
}

fn complete_flags(context: &CursorContext, items: &mut Vec<CompletionItem>) {
    for flag in ["--help", "--version", "--verbose", "--force", "--all"] {
        if prefix_match(flag, &context.prefix) {
            items.push(CompletionItem {
                label: flag.to_owned(),
                replacement: flag.to_owned(),
                range: context.range.clone(),
                kind: CompletionKind::Flag,
                description: "common flag".to_owned(),
                score: score(flag, &context.prefix),
            });
        }
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn split_path_prefix(cwd: &Path, prefix: &str) -> (PathBuf, String) {
    if prefix == "." || prefix == ".." {
        return (cwd.to_path_buf(), prefix.to_owned());
    }
    let expanded = prefix.strip_prefix('~').map(|rest| {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf())
            .join(rest.trim_start_matches(std::path::MAIN_SEPARATOR))
    });
    let path = expanded.unwrap_or_else(|| cwd.join(prefix));
    if prefix.ends_with(std::path::MAIN_SEPARATOR) {
        return (path, String::new());
    }
    let base = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.to_path_buf());
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    (base, name)
}

fn prefix_match(label: &str, prefix: &str) -> bool {
    label
        .to_ascii_lowercase()
        .starts_with(&prefix.to_ascii_lowercase())
}

fn score(label: &str, prefix: &str) -> i64 {
    if label == prefix {
        100
    } else if label
        .to_ascii_lowercase()
        .starts_with(&prefix.to_ascii_lowercase())
    {
        70 - i64::try_from(label.len()).unwrap_or(0).min(50)
    } else {
        0
    }
}

fn shell_escape(value: &str, path_context: bool) -> String {
    let needs_escape = value.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '"' | '$' | '&' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
            )
    });
    if !needs_escape {
        return value.to_owned();
    }
    if path_context {
        value
            .replace(' ', "\\ ")
            .replace('"', "\\\"")
            .replace('\'', "\\'")
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn escape_for_context(value: &str, quote: QuoteMode, path_context: bool) -> String {
    match quote {
        QuoteMode::None => shell_escape(value, path_context),
        QuoteMode::Single => value.replace('\'', "'\\''"),
        QuoteMode::Double => value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn path_completion_escapes_spaces() {
        let dir = env::temp_dir().join(format!("terminaste-completion-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("two words.txt"), "x").unwrap();
        let request = CompletionRequest {
            input: "cat two".to_owned(),
            cursor: 7,
            cwd: dir.clone(),
            path_dirs: Vec::new(),
            aliases: Vec::new(),
            revision: 1,
        };
        let items = complete(&request);
        assert!(items
            .iter()
            .any(|item| item.replacement == "two\\ words.txt"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn path_completion_inside_double_quotes_keeps_spaces_literal() {
        let dir = env::temp_dir().join(format!(
            "terminaste-completion-quotes-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("two words.txt"), "x").unwrap();
        let request = CompletionRequest {
            input: "cat \"two".to_owned(),
            cursor: 8,
            cwd: dir.clone(),
            path_dirs: Vec::new(),
            aliases: Vec::new(),
            revision: 1,
        };
        let items = complete(&request);
        assert!(items.iter().any(|item| item.replacement == "two words.txt"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn escaped_space_stays_in_current_token() {
        let dir = env::temp_dir().join(format!(
            "terminaste-completion-escaped-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("two words.txt"), "x").unwrap();
        let request = CompletionRequest {
            input: "cat two\\ wo".to_owned(),
            cursor: 11,
            cwd: dir.clone(),
            path_dirs: Vec::new(),
            aliases: Vec::new(),
            revision: 1,
        };
        let items = complete(&request);
        assert!(items
            .iter()
            .any(|item| item.replacement == "two\\ words.txt"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dotfiles_are_hidden_until_prefix_starts_with_dot() {
        let dir = env::temp_dir().join(format!(
            "terminaste-completion-dotfiles-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".env"), "x").unwrap();
        let hidden_request = CompletionRequest {
            input: "cat ".to_owned(),
            cursor: 4,
            cwd: dir.clone(),
            path_dirs: Vec::new(),
            aliases: Vec::new(),
            revision: 1,
        };
        assert!(complete(&hidden_request)
            .iter()
            .all(|item| item.label != ".env"));

        let visible_request = CompletionRequest {
            input: "cat .".to_owned(),
            cursor: 5,
            cwd: dir.clone(),
            path_dirs: Vec::new(),
            aliases: Vec::new(),
            revision: 2,
        };
        assert!(complete(&visible_request)
            .iter()
            .any(|item| item.label == ".env"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directory_completion_appends_separator() {
        let dir = env::temp_dir().join(format!(
            "terminaste-completion-directory-{}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join("src")).unwrap();
        let request = CompletionRequest {
            input: "cd s".to_owned(),
            cursor: 4,
            cwd: dir.clone(),
            path_dirs: Vec::new(),
            aliases: Vec::new(),
            revision: 1,
        };
        let items = complete(&request);
        assert!(items.iter().any(|item| {
            item.label == "src" && item.replacement.ends_with(std::path::MAIN_SEPARATOR)
        }));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_revision_is_dropped() {
        let request = CompletionRequest {
            input: "ec".to_owned(),
            cursor: 2,
            cwd: PathBuf::from("."),
            path_dirs: Vec::new(),
            aliases: vec![("echo".to_owned(), "echo".to_owned())],
            revision: 1,
        };
        assert!(complete_if_current(&request, 2).is_none());
        assert_eq!(complete_if_current(&request, 1).unwrap().revision, 1);
    }

    #[test]
    fn accepting_replaces_only_range() {
        let item = CompletionItem {
            label: "src".to_owned(),
            replacement: "src/".to_owned(),
            range: 3..4,
            kind: CompletionKind::Directory,
            description: String::new(),
            score: 1,
        };
        assert_eq!(apply_completion("cd s && pwd", &item), "cd src/ && pwd");
    }
}
