//! What `:open:` does with a picked text: which URL, folder or file it
//! stands for, and how that is safe to open.

use std::fmt;
use std::path::{Path, PathBuf};

use regex::Regex;
use thiserror::Error;

use super::links::{self, LINK_PATTERN};
use super::patterns::BUILTIN_PATTERNS;

/// A URL with `{}` where the picked text goes, such as
/// `https://example.atlassian.net/browse/{}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlTemplate(String);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlTemplateError {
    #[error("open_url must start with https:// or http://, got \"{0}\"")]
    Scheme(String),
    #[error("open_url must contain {{}} where the match goes, got \"{0}\"")]
    NoPlaceholder(String),
}

impl UrlTemplate {
    pub fn parse(template: &str) -> Result<Self, UrlTemplateError> {
        if !(template.starts_with("https://") || template.starts_with("http://")) {
            return Err(UrlTemplateError::Scheme(template.to_string()));
        }
        if !template.contains("{}") {
            return Err(UrlTemplateError::NoPlaceholder(template.to_string()));
        }
        Ok(UrlTemplate(template.to_string()))
    }

    /// The URL for `text`, percent-encoded so it cannot leave its slot.
    pub fn expand(&self, text: &str) -> String {
        self.0.replace("{}", &percent_encode(text))
    }
}

fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The command line that edits a file, as words: `{path}` is a word of its
/// own, `{line}` may sit inside one (`+{line}`); a word with `{line}` is
/// dropped when there is no line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    template: String,
    words: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EditorError {
    #[error("editor is empty")]
    Empty,
    #[error("editor: {{path}} must be a word of its own, got \"{0}\"")]
    PathInsideWord(String),
}

const PATH_PLACEHOLDER: &str = "{path}";
const LINE_PLACEHOLDER: &str = "{line}";

impl EditorCommand {
    /// `template` is the line as written, `words` the same line split into
    /// words. Without placeholders the editor gets `+{line} {path}`.
    pub fn new(template: &str, mut words: Vec<String>) -> Result<Self, EditorError> {
        if words.is_empty() {
            return Err(EditorError::Empty);
        }
        if let Some(word) = words
            .iter()
            .find(|word| word.contains(PATH_PLACEHOLDER) && *word != PATH_PLACEHOLDER)
        {
            return Err(EditorError::PathInsideWord(word.clone()));
        }
        let has_line = words.iter().any(|word| word.contains(LINE_PLACEHOLDER));
        if !words.iter().any(|word| word == PATH_PLACEHOLDER) {
            if !has_line {
                words.push(format!("+{LINE_PLACEHOLDER}"));
            }
            words.push(PATH_PLACEHOLDER.to_string());
        }
        Ok(EditorCommand {
            template: template.to_string(),
            words,
        })
    }

    /// The line as written, to hand to the process that runs the editor.
    pub fn template(&self) -> &str {
        &self.template
    }

    pub fn argv(&self, path: &str, line: Option<u32>) -> Vec<String> {
        self.words
            .iter()
            .filter_map(|word| {
                if word == PATH_PLACEHOLDER {
                    return Some(path.to_string());
                }
                if word.contains(LINE_PLACEHOLDER) {
                    return line.map(|line| word.replace(LINE_PLACEHOLDER, &line.to_string()));
                }
                Some(word.clone())
            })
            .collect()
    }
}

/// What the file system says about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Missing,
    Directory,
    /// A regular file with an execute bit.
    Executable,
    File,
}

/// A file and the line to put the cursor on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLocation {
    pub path: PathBuf,
    pub line: Option<u32>,
}

/// Why nothing is opened; the text is the notice the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Scheme(String),
    NotFound(PathBuf),
    /// The relative path exists under more than one search root.
    Ambiguous {
        path: String,
        places: usize,
    },
    /// The search roots name more folders than are looked in, so a single
    /// hit may not be the only one.
    TooManyFolders(String),
    NothingToOpen,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Scheme(scheme) => write!(f, "won't open {scheme} links"),
            Refusal::NotFound(path) => write!(f, "not found: {}", path.display()),
            Refusal::Ambiguous { path, places } => {
                write!(f, "found in {places} places; not guessing which: {path}")
            }
            Refusal::TooManyFolders(path) => write!(
                f,
                "search_roots name over {MAX_SEARCH_DIRS} folders; not guessing: {path}"
            ),
            Refusal::NothingToOpen => write!(f, "nothing to open"),
        }
    }
}

/// What opening a picked text comes down to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// An http(s) URL for the system opener.
    Url(String),
    /// A folder for the system opener.
    Directory(PathBuf),
    /// Something the system opener would run: show it in its folder instead.
    Reveal(PathBuf),
    Edit(FileLocation),
    /// A commit with no GitHub page to show: copy the SHA.
    CopySha(String),
    Refused(Refusal),
}

/// The pattern name whose matches are Git commits.
pub const SHA_PATTERN: &str = "sha";

/// Patterns whose matches are file names even without a slash.
const PATH_PATTERNS: &[&str] = &["path", "git-status", "diff"];

/// The system opener runs or installs these, so they are only ever revealed.
/// An editor runs nothing, so scripts are not among them.
const RUN_BY_OPENER: &[&str] = &[
    "app",
    "workflow",
    "pkg",
    "mpkg",
    "dmg",
    "prefpane",
    "saver",
    "appex",
    "qlgenerator",
    "kext",
    "action",
];

/// Folders the system opener would hand to a program rather than show.
const LAUNCHABLE_FOLDER_EXTENSIONS: &[&str] = &["command", "terminal", "tool", "sh"];

/// Screen matches looked at for clues, nearest first; the rest are ignored.
const MAX_SCREEN_CLUES: usize = 32;
/// Folders from `search_roots` a relative path is looked for in.
const MAX_SEARCH_DIRS: usize = 256;

/// A folder from `search_roots` that relative paths are looked for in:
/// `~/dev`, or `~/dev/*` for every folder directly inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRoot {
    dir: String,
    each_subdirectory: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SearchRootError {
    #[error("search_roots entries must start with / or ~/, got \"{0}\"")]
    NotAbsolute(String),
    #[error("search_roots entries may only have * as their last part, got \"{0}\"")]
    Wildcard(String),
}

impl SearchRoot {
    pub fn parse(text: &str) -> Result<Self, SearchRootError> {
        let (dir, each_subdirectory) = match text.strip_suffix("/*") {
            Some("") => ("/", true),
            Some(dir) => (dir, true),
            None => (text, false),
        };
        if dir.contains(['*', '?', '[']) {
            return Err(SearchRootError::Wildcard(text.to_string()));
        }
        if !(dir.starts_with('/') || is_home_relative(dir)) {
            return Err(SearchRootError::NotAbsolute(text.to_string()));
        }
        let dir = match dir.trim_end_matches('/') {
            "" => "/",
            trimmed => trimmed,
        };
        Ok(SearchRoot {
            dir: dir.to_string(),
            each_subdirectory,
        })
    }

    /// `~/dev/*`.
    pub fn default_roots() -> Vec<SearchRoot> {
        vec![SearchRoot {
            dir: "~/dev".to_string(),
            each_subdirectory: true,
        }]
    }
}

/// Another match on the visible screen: a clue to where a relative path or
/// a commit belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenMatch<'a> {
    pub text: &'a str,
    pub row: usize,
}

/// Everything about one picked text the resolver needs.
pub struct OpenRequest<'a> {
    pub text: &'a str,
    pub pattern: &'a str,
    pub url_template: Option<&'a UrlTemplate>,
    /// The source pane's working directory.
    pub cwd: Option<&'a Path>,
    pub home: Option<&'a Path>,
    /// The screen row the picked text starts on.
    pub row: usize,
    /// Every match on the visible screen.
    pub screen: &'a [ScreenMatch<'a>],
    pub search_roots: &'a [SearchRoot],
}

/// What the resolver asks of the machine it runs on.
pub trait Surroundings {
    fn kind(&self, path: &Path) -> PathKind;
    /// Where `path` leads once every link in it is followed.
    fn real_path(&self, path: &Path) -> PathBuf;
    /// The `origin` remote URL of the Git repository containing `dir`.
    fn git_origin(&self, dir: &Path) -> Option<String>;
    /// The folders directly inside `dir`, sorted; at most `at_most` of them,
    /// and fewer only when that is all there are.
    fn subdirectories(&self, dir: &Path, at_most: usize) -> Vec<PathBuf>;
}

pub fn resolve<S: Surroundings + ?Sized>(
    request: &OpenRequest<'_>,
    surroundings: &S,
) -> OpenTarget {
    let text = request.text;
    if let Some(template) = request.url_template {
        return OpenTarget::Url(template.expand(text));
    }
    // A link the host reported names a file it was drawn for; a `file://`
    // URL merely printed on screen is still refused below.
    if request.pattern == LINK_PATTERN
        && let Some(path) = links::file_url_path(text)
    {
        let as_path = OpenRequest {
            text: &path,
            pattern: "path",
            ..*request
        };
        return resolve(&as_path, surroundings);
    }
    if let Some(scheme) = link_scheme(text) {
        return if scheme == "http" || scheme == "https" {
            OpenTarget::Url(text.to_string())
        } else {
            OpenTarget::Refused(Refusal::Scheme(scheme))
        };
    }
    // The sha pattern also catches long numbers (timestamps, ids); a commit
    // that is all digits is rare enough to leave to a copy.
    if request.pattern == SHA_PATTERN && text.bytes().any(|byte| byte.is_ascii_alphabetic()) {
        return commit_repository(request, surroundings).map_or_else(
            || OpenTarget::CopySha(text.to_string()),
            |repository| OpenTarget::Url(format!("{repository}/commit/{text}")),
        );
    }
    let looks_like_a_path =
        PATH_PATTERNS.contains(&request.pattern) || text.contains('/') || text.starts_with('~');
    // A number, address or id a built-in pattern caught names no file, even
    // where some folder happens to share it; only custom patterns hold bare
    // file names.
    let builtin = BUILTIN_PATTERNS
        .iter()
        .any(|&(name, _)| name == request.pattern);
    if !looks_like_a_path && builtin {
        return OpenTarget::Refused(Refusal::NothingToOpen);
    }
    match locate(request, surroundings) {
        Located::Found(path, kind, line) => open_found(path, kind, line, surroundings),
        Located::Ambiguous { path, places } => {
            OpenTarget::Refused(Refusal::Ambiguous { path, places })
        }
        Located::Unsure(path) => OpenTarget::Refused(Refusal::TooManyFolders(path)),
        Located::Missing(path) if looks_like_a_path => OpenTarget::Refused(Refusal::NotFound(path)),
        Located::Missing(_) => OpenTarget::Refused(Refusal::NothingToOpen),
    }
}

/// `https` in `https://…`, `git@` for SCP-style Git remotes.
fn link_scheme(text: &str) -> Option<String> {
    if text.starts_with("git@") {
        return Some("git@".to_string());
    }
    let (scheme, _) = text.split_once("://")?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// `https://github.com/owner/repo` for the GitHub remote forms Git prints.
pub fn github_repository(origin: &str) -> Option<String> {
    let rest = origin
        .strip_prefix("git@github.com:")
        .or_else(|| origin.strip_prefix("ssh://git@github.com/"))
        .or_else(|| origin.strip_prefix("https://github.com/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repository) = rest.split_once('/')?;
    let simple = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (simple(owner) && simple(repository))
        .then(|| format!("https://github.com/{owner}/{repository}"))
}

/// The first part of GitHub pages that belong to no repository, such as
/// `https://github.com/settings/tokens`.
const GITHUB_SITE_PAGES: &[&str] = &[
    "about",
    "apps",
    "codespaces",
    "collections",
    "enterprise",
    "enterprises",
    "explore",
    "features",
    "issues",
    "login",
    "logout",
    "marketplace",
    "new",
    "notifications",
    "organizations",
    "orgs",
    "pricing",
    "pulls",
    "search",
    "security",
    "settings",
    "site",
    "sponsors",
    "topics",
    "trending",
    "users",
];

/// The repository a web link into GitHub points at, such as
/// `https://github.com/o/r/pull/12`.
fn github_link_repository(link: &str) -> Option<String> {
    let rest = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_prefix("github.com/")?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = rest.split('/');
    let (owner, repository) = (parts.next()?, parts.next()?);
    if GITHUB_SITE_PAGES.contains(&owner.to_ascii_lowercase().as_str()) {
        return None;
    }
    github_repository(&format!("https://github.com/{owner}/{repository}"))
}

/// The GitHub repository a commit picked from the screen belongs to: the
/// pane's own, else the nearest one the screen names.
fn commit_repository<S: Surroundings + ?Sized>(
    request: &OpenRequest<'_>,
    surroundings: &S,
) -> Option<String> {
    let github_origin = |dir: &Path| {
        surroundings
            .git_origin(dir)
            .and_then(|origin| github_repository(&origin))
    };
    if let Some(repository) = request.cwd.and_then(github_origin) {
        return Some(repository);
    }
    // Each run of `git` is a process, so a repository is asked once and a
    // folder outside any repository not at all.
    let mut asked: Vec<PathBuf> = request
        .cwd
        .and_then(|cwd| git_root(cwd, surroundings))
        .into_iter()
        .collect();
    let clues = nearest_clues(request, |text| {
        is_anchored(text) || github_link_repository(text).is_some()
    });
    clues.into_iter().find_map(|clue| {
        if let Some(repository) = github_link_repository(clue.text) {
            return Some(repository);
        }
        let (path, kind) = named_by(clue.text, request.home, surroundings)?;
        let root = repository_holding(&path, kind, surroundings)?;
        if asked.contains(&root) {
            return None;
        }
        let repository = github_origin(&root);
        asked.push(root);
        repository
    })
}

enum Located {
    Found(PathBuf, PathKind, Option<u32>),
    Ambiguous {
        path: String,
        places: usize,
    },
    /// In one search folder, but not every search folder was looked in.
    Unsure(String),
    /// Nowhere; the path to report.
    Missing(PathBuf),
}

/// Finds what the picked text names. A relative path is looked for in the
/// pane's directory, its Git root, the folders the screen names (nearest
/// first) and, when exactly one has it, the search roots.
fn locate<S: Surroundings + ?Sized>(request: &OpenRequest<'_>, surroundings: &S) -> Located {
    let spellings = spellings(request.text);
    let (written, _) = spellings[0];
    let missing =
        || absolute(written, request.cwd, request.home).unwrap_or_else(|| PathBuf::from(written));
    if is_anchored(written) {
        return spellings
            .iter()
            .find_map(|&(written, line)| {
                let path = absolute(written, None, request.home)?;
                existing(path, line, surroundings)
            })
            .map_or_else(|| Located::Missing(missing()), found);
    }
    let mut tried: Vec<PathBuf> = Vec::new();
    let mut try_in = |dir: PathBuf| {
        if tried.contains(&dir) {
            return None;
        }
        let hit = find_under(&dir, &spellings, surroundings);
        tried.push(dir);
        hit
    };
    if let Some(cwd) = request.cwd.filter(|cwd| cwd.is_absolute()) {
        if let Some(hit) = try_in(cwd.to_path_buf()) {
            return found(hit);
        }
        if let Some(hit) = git_root(cwd, surroundings).and_then(&mut try_in) {
            return found(hit);
        }
    }
    let screen_folders = nearest_clues(request, is_anchored)
        .into_iter()
        .flat_map(|clue| folders_named_by(clue.text, request.home, surroundings));
    for dir in screen_folders {
        if let Some(hit) = try_in(dir) {
            return found(hit);
        }
    }
    let (dirs, complete) = search_dirs(request, surroundings);
    let mut hits: Vec<(PathBuf, PathKind, Option<u32>)> = Vec::new();
    for dir in dirs {
        if let Some(hit) = find_under(&dir, &spellings, surroundings) {
            let real = surroundings.real_path(&hit.0);
            if !hits
                .iter()
                .any(|(path, ..)| surroundings.real_path(path) == real)
            {
                hits.push(hit);
            }
        }
    }
    match hits.len() {
        0 => Located::Missing(missing()),
        1 if !complete => Located::Unsure(written.to_string()),
        1 => found(hits.remove(0)),
        places => Located::Ambiguous {
            path: written.to_string(),
            places,
        },
    }
}

fn found((path, kind, line): (PathBuf, PathKind, Option<u32>)) -> Located {
    Located::Found(path, kind, line)
}

/// The ways a picked path may be meant, each with its line: as written,
/// without the `a/` or `b/` of a diff, and without a full stop that ends a
/// sentence.
fn spellings(text: &str) -> Vec<(&str, Option<u32>)> {
    let mut spellings = Vec::new();
    let without_full_stop = text.trim_end_matches('.');
    let mut wholes = vec![text];
    if !without_full_stop.is_empty() && without_full_stop != text {
        wholes.push(without_full_stop);
    }
    for whole in wholes {
        let (written, line) = split_line_suffix(whole);
        spellings.push((written, line));
        if let Some(unprefixed) = written
            .strip_prefix("a/")
            .or_else(|| written.strip_prefix("b/"))
        {
            spellings.push((unprefixed, line));
        }
    }
    spellings
}

fn find_under<S: Surroundings + ?Sized>(
    dir: &Path,
    spellings: &[(&str, Option<u32>)],
    surroundings: &S,
) -> Option<(PathBuf, PathKind, Option<u32>)> {
    spellings.iter().find_map(|&(written, line)| {
        let path = absolute(written, Some(dir), None)?;
        existing(path, line, surroundings)
    })
}

fn existing<S: Surroundings + ?Sized>(
    path: PathBuf,
    line: Option<u32>,
    surroundings: &S,
) -> Option<(PathBuf, PathKind, Option<u32>)> {
    match surroundings.kind(&path) {
        PathKind::Missing => None,
        kind => Some((path, kind, line)),
    }
}

/// `/…`, `~` or `~/…`: a path that does not depend on a working directory.
fn is_anchored(text: &str) -> bool {
    text.starts_with('/') || is_home_relative(text)
}

fn is_home_relative(text: &str) -> bool {
    text == "~" || text.starts_with("~/")
}

/// The screen matches `relevant` keeps, nearest to the picked text first:
/// the same row, then the rows above, then the rows below.
fn nearest_clues<'a>(
    request: &OpenRequest<'a>,
    relevant: impl Fn(&str) -> bool,
) -> Vec<ScreenMatch<'a>> {
    let distance = |row: usize| {
        if row == request.row {
            (0, 0)
        } else if row < request.row {
            (1, request.row - row)
        } else {
            (2, row - request.row)
        }
    };
    let mut clues: Vec<ScreenMatch<'a>> = request
        .screen
        .iter()
        .copied()
        .filter(|clue| relevant(clue.text))
        .collect();
    clues.sort_by_key(|clue| distance(clue.row));
    clues.truncate(MAX_SCREEN_CLUES);
    clues
}

/// The folders an absolute or `~` path on screen points at: the path itself
/// when it is a folder, and the Git root of whatever it names.
fn folders_named_by<S: Surroundings + ?Sized>(
    text: &str,
    home: Option<&Path>,
    surroundings: &S,
) -> Vec<PathBuf> {
    let Some((path, kind)) = named_by(text, home, surroundings) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    if let Some(root) = repository_holding(&path, kind, surroundings)
        && root != path
    {
        folders.push(root);
    }
    if kind == PathKind::Directory {
        folders.insert(0, path);
    }
    folders
}

/// What an absolute or `~` path on screen names, when it exists.
fn named_by<S: Surroundings + ?Sized>(
    text: &str,
    home: Option<&Path>,
    surroundings: &S,
) -> Option<(PathBuf, PathKind)> {
    spellings(text).iter().find_map(|&(written, line)| {
        let path = absolute(written, None, home)?;
        let (path, kind, _) = existing(path, line, surroundings)?;
        Some((path, kind))
    })
}

/// The Git root of the repository a file or folder is in.
fn repository_holding<S: Surroundings + ?Sized>(
    path: &Path,
    kind: PathKind,
    surroundings: &S,
) -> Option<PathBuf> {
    let folder = if kind == PathKind::Directory {
        Some(path)
    } else {
        path.parent()
    };
    folder.and_then(|folder| git_root(folder, surroundings))
}

/// The nearest folder at or above `dir` holding a `.git`.
fn git_root<S: Surroundings + ?Sized>(dir: &Path, surroundings: &S) -> Option<PathBuf> {
    dir.ancestors()
        .find(|folder| surroundings.kind(&folder.join(".git")) != PathKind::Missing)
        .map(Path::to_path_buf)
}

/// The folders the search roots name, at most [`MAX_SEARCH_DIRS`] of them,
/// and whether that is all of them.
fn search_dirs<S: Surroundings + ?Sized>(
    request: &OpenRequest<'_>,
    surroundings: &S,
) -> (Vec<PathBuf>, bool) {
    let mut dirs = Vec::new();
    for root in request.search_roots {
        let Some(base) = absolute(&root.dir, None, request.home) else {
            continue;
        };
        if root.each_subdirectory {
            // One past the cap tells a full list from a cut one.
            let room = MAX_SEARCH_DIRS + 1 - dirs.len();
            dirs.extend(surroundings.subdirectories(&base, room));
        } else {
            dirs.push(base);
        }
        if dirs.len() > MAX_SEARCH_DIRS {
            break;
        }
    }
    let complete = dirs.len() <= MAX_SEARCH_DIRS;
    dirs.truncate(MAX_SEARCH_DIRS);
    (dirs, complete)
}

fn open_found<S: Surroundings + ?Sized>(
    path: PathBuf,
    kind: PathKind,
    line: Option<u32>,
    surroundings: &S,
) -> OpenTarget {
    // The opener follows links, so a link named `docs` to an app bundle
    // would launch it: what the path leads to counts too.
    let real = surroundings.real_path(&path);
    let either =
        |extensions: &[&str]| has_extension(&path, extensions) || has_extension(&real, extensions);
    match kind {
        PathKind::Missing => OpenTarget::Refused(Refusal::NotFound(path)),
        _ if either(RUN_BY_OPENER) => OpenTarget::Reveal(path),
        PathKind::Directory if either(LAUNCHABLE_FOLDER_EXTENSIONS) => OpenTarget::Reveal(path),
        PathKind::Directory => OpenTarget::Directory(path),
        PathKind::File | PathKind::Executable => OpenTarget::Edit(FileLocation { path, line }),
    }
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extensions.contains(&extension.to_ascii_lowercase().as_str()))
}

/// `src/x.rs:12:3` is `src/x.rs` at line 12; the column is dropped.
fn split_line_suffix(text: &str) -> (&str, Option<u32>) {
    let suffix = Regex::new(r":(\d+)(?::\d+)?:?$").expect("valid regex");
    match suffix.captures(text) {
        Some(captures) => {
            let start = captures.get(0).expect("group 0").start();
            let line = captures[1].parse().ok();
            if start == 0 {
                (text, None)
            } else {
                (&text[..start], line)
            }
        }
        None => (text, None),
    }
}

/// Every path handed onwards is absolute, so none can start with `-` and be
/// read as an option by the program that opens it.
fn absolute(written: &str, cwd: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let expanded = if written == "~" {
        home?.to_path_buf()
    } else if let Some(rest) = written.strip_prefix("~/") {
        home?.join(rest)
    } else {
        PathBuf::from(written)
    };
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        cwd.filter(|cwd| cwd.is_absolute())?.join(expanded)
    };
    // Rebuilding from components drops the `.` segments of `./run`.
    Some(joined.components().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Machine {
        kinds: BTreeMap<PathBuf, PathKind>,
        links: BTreeMap<PathBuf, PathBuf>,
        origins: BTreeMap<PathBuf, String>,
        git_runs: Cell<usize>,
    }

    impl Machine {
        fn with(mut self, path: &str, kind: PathKind) -> Self {
            self.kinds.insert(PathBuf::from(path), kind);
            self
        }

        fn linking(mut self, link: &str, to: &str) -> Self {
            self.links.insert(PathBuf::from(link), PathBuf::from(to));
            self
        }

        fn with_origin(self, origin: &str) -> Self {
            self.with_origin_at("/work/repo", origin)
        }

        fn with_origin_at(mut self, repository: &str, origin: &str) -> Self {
            self.origins
                .insert(PathBuf::from(repository), origin.to_string());
            self
        }

        fn with_repository(self, root: &str) -> Self {
            self.with(root, PathKind::Directory)
                .with(&format!("{root}/.git"), PathKind::Directory)
        }
    }

    impl Surroundings for Machine {
        fn kind(&self, path: &Path) -> PathKind {
            self.kinds.get(path).copied().unwrap_or(PathKind::Missing)
        }

        fn real_path(&self, path: &Path) -> PathBuf {
            self.links
                .get(path)
                .cloned()
                .unwrap_or_else(|| path.to_path_buf())
        }

        fn git_origin(&self, dir: &Path) -> Option<String> {
            self.git_runs.set(self.git_runs.get() + 1);
            dir.ancestors()
                .find_map(|folder| self.origins.get(folder).cloned())
        }

        fn subdirectories(&self, dir: &Path, at_most: usize) -> Vec<PathBuf> {
            self.kinds
                .iter()
                .filter(|(path, kind)| **kind == PathKind::Directory && path.parent() == Some(dir))
                .map(|(path, _)| path.clone())
                .take(at_most)
                .collect()
        }
    }

    struct Ask<'a> {
        text: &'a str,
        pattern: &'a str,
        cwd: &'a str,
        row: usize,
        screen: Vec<ScreenMatch<'a>>,
        search_roots: Vec<SearchRoot>,
    }

    impl<'a> Ask<'a> {
        fn new(text: &'a str, pattern: &'a str) -> Self {
            Ask {
                text,
                pattern,
                cwd: "/work/repo",
                row: 0,
                screen: Vec::new(),
                search_roots: Vec::new(),
            }
        }

        fn from(mut self, cwd: &'a str) -> Self {
            self.cwd = cwd;
            self
        }

        fn at_row(mut self, row: usize) -> Self {
            self.row = row;
            self
        }

        fn beside(mut self, text: &'a str, row: usize) -> Self {
            self.screen.push(ScreenMatch { text, row });
            self
        }

        fn searching(mut self, roots: &[&str]) -> Self {
            self.search_roots = roots
                .iter()
                .map(|root| SearchRoot::parse(root).unwrap())
                .collect();
            self
        }

        fn resolve(&self, machine: &Machine) -> OpenTarget {
            resolve(
                &OpenRequest {
                    text: self.text,
                    pattern: self.pattern,
                    url_template: None,
                    cwd: Some(Path::new(self.cwd)),
                    home: Some(Path::new("/home/me")),
                    row: self.row,
                    screen: &self.screen,
                    search_roots: &self.search_roots,
                },
                machine,
            )
        }
    }

    fn open(text: &str, pattern: &str, machine: &Machine) -> OpenTarget {
        Ask::new(text, pattern).resolve(machine)
    }

    fn edit(path: &str, line: Option<u32>) -> OpenTarget {
        OpenTarget::Edit(FileLocation {
            path: path.into(),
            line,
        })
    }

    #[test]
    fn a_pattern_url_template_gets_the_percent_encoded_match() {
        let template = UrlTemplate::parse("https://example.atlassian.net/browse/{}").unwrap();
        let request = OpenRequest {
            text: "RET-1234",
            pattern: "jira",
            url_template: Some(&template),
            cwd: None,
            home: None,
            row: 0,
            screen: &[],
            search_roots: &[],
        };
        assert_eq!(
            resolve(&request, &Machine::default()),
            OpenTarget::Url("https://example.atlassian.net/browse/RET-1234".into())
        );
        assert_eq!(
            template.expand("a b/c?d=$(x)&é"),
            "https://example.atlassian.net/browse/a%20b%2Fc%3Fd%3D%24%28x%29%26%C3%A9"
        );
    }

    #[test]
    fn a_url_template_must_be_http_and_have_a_placeholder() {
        assert!(UrlTemplate::parse("http://x/{}").is_ok());
        assert!(matches!(
            UrlTemplate::parse("file:///{}"),
            Err(UrlTemplateError::Scheme(_))
        ));
        assert!(matches!(
            UrlTemplate::parse("javascript:{}"),
            Err(UrlTemplateError::Scheme(_))
        ));
        assert!(matches!(
            UrlTemplate::parse("https://x/browse"),
            Err(UrlTemplateError::NoPlaceholder(_))
        ));
    }

    #[test]
    fn a_file_link_from_link_mode_opens_as_the_path_it_names() {
        let machine = Machine::default()
            .with("/work/My Notes/a.md", PathKind::File)
            .with("/Applications/Calc.app", PathKind::Directory);
        assert_eq!(
            open("file:///work/My%20Notes/a.md", LINK_PATTERN, &machine),
            edit("/work/My Notes/a.md", None)
        );
        assert_eq!(
            open("file:///Applications/Calc.app", LINK_PATTERN, &machine),
            OpenTarget::Reveal("/Applications/Calc.app".into())
        );
        assert_eq!(
            open("file://server/share/a.md", LINK_PATTERN, &machine),
            OpenTarget::Refused(Refusal::Scheme("file".into()))
        );
        assert_eq!(
            open("ssh://host/x", LINK_PATTERN, &machine),
            OpenTarget::Refused(Refusal::Scheme("ssh".into()))
        );
    }

    #[test]
    fn web_links_open_and_other_schemes_are_refused() {
        let machine = Machine::default();
        assert_eq!(
            open("https://x.io/a?b=1", "url", &machine),
            OpenTarget::Url("https://x.io/a?b=1".into())
        );
        assert_eq!(
            open("HTTP://x.io", "url", &machine),
            OpenTarget::Url("HTTP://x.io".into())
        );
        for (link, scheme) in [
            ("file:///Applications/Calculator.app", "file"),
            ("ssh://host/x", "ssh"),
            ("ftp://host/x", "ftp"),
            ("git://host/x", "git"),
            ("git@github.com:o/r.git", "git@"),
        ] {
            assert_eq!(
                open(link, "url", &machine),
                OpenTarget::Refused(Refusal::Scheme(scheme.into())),
                "{link}"
            );
        }
        assert_eq!(
            Refusal::Scheme("file".into()).to_string(),
            "won't open file links"
        );
    }

    #[test]
    fn a_sha_opens_its_github_commit_page() {
        for origin in [
            "git@github.com:o/r.git",
            "git@github.com:o/r",
            "ssh://git@github.com/o/r.git",
            "https://github.com/o/r.git",
            "https://github.com/o/r",
        ] {
            let machine = Machine::default().with_origin(origin);
            assert_eq!(
                open("8b1a995", SHA_PATTERN, &machine),
                OpenTarget::Url("https://github.com/o/r/commit/8b1a995".into()),
                "{origin}"
            );
        }
    }

    #[test]
    fn a_sha_outside_a_github_repository_is_copied() {
        for machine in [
            Machine::default(),
            Machine::default().with_origin("git@gitlab.com:o/r.git"),
            Machine::default().with_origin("https://github.com.evil.io/o/r"),
            Machine::default().with_origin("https://github.com/o/r/../x"),
        ] {
            assert_eq!(
                open("8b1a995", SHA_PATTERN, &machine),
                OpenTarget::CopySha("8b1a995".into())
            );
        }
    }

    #[test]
    fn a_number_the_sha_pattern_caught_is_not_a_commit() {
        let machine = Machine::default().with_origin("git@github.com:o/r.git");
        assert_eq!(
            open("1700000000", SHA_PATTERN, &machine),
            OpenTarget::Refused(Refusal::NothingToOpen)
        );
    }

    #[test]
    fn a_path_ending_a_sentence_drops_the_full_stop_when_needed() {
        let machine = Machine::default()
            .with("/home/me/notes/todo.md", PathKind::File)
            .with("/work/repo/src/x.rs", PathKind::File);
        assert_eq!(
            open("~/notes/todo.md.", "path", &machine),
            edit("/home/me/notes/todo.md", None)
        );
        assert_eq!(
            open("src/x.rs:12.", "path", &machine),
            edit("/work/repo/src/x.rs", Some(12))
        );
        assert_eq!(
            open("src/gone.rs.", "path", &machine),
            OpenTarget::Refused(Refusal::NotFound("/work/repo/src/gone.rs.".into()))
        );
    }

    #[test]
    fn each_spelling_of_a_path_is_tried_once() {
        assert_eq!(spellings("src/x.rs:12"), vec![("src/x.rs", Some(12))]);
        assert_eq!(
            spellings("a/x.rs."),
            vec![
                ("a/x.rs.", None),
                ("x.rs.", None),
                ("a/x.rs", None),
                ("x.rs", None)
            ]
        );
    }

    #[test]
    fn a_relative_file_opens_in_the_editor_at_its_line() {
        let machine = Machine::default().with("/work/repo/src/x.rs", PathKind::File);
        assert_eq!(
            open("src/x.rs", "path", &machine),
            edit("/work/repo/src/x.rs", None)
        );
        assert_eq!(
            open("src/x.rs:12:3", "path", &machine),
            edit("/work/repo/src/x.rs", Some(12))
        );
        assert_eq!(
            open("src/x.rs:12", "path", &machine),
            edit("/work/repo/src/x.rs", Some(12))
        );
    }

    #[test]
    fn a_diff_prefix_is_dropped_when_the_prefixed_path_does_not_exist() {
        let machine = Machine::default().with("/work/repo/src/x.rs", PathKind::File);
        assert_eq!(
            open("a/src/x.rs", "path", &machine),
            edit("/work/repo/src/x.rs", None)
        );
        assert_eq!(
            open("b/src/x.rs:4", "path", &machine),
            edit("/work/repo/src/x.rs", Some(4))
        );
    }

    #[test]
    fn a_tilde_is_the_home_directory() {
        let machine = Machine::default().with("/home/me/notes", PathKind::Directory);
        assert_eq!(
            open("~/notes", "path", &machine),
            OpenTarget::Directory("/home/me/notes".into())
        );
    }

    #[test]
    fn a_missing_path_is_reported_and_nothing_opens() {
        assert_eq!(
            open("src/gone.rs", "path", &Machine::default()),
            OpenTarget::Refused(Refusal::NotFound("/work/repo/src/gone.rs".into()))
        );
        assert_eq!(Refusal::NotFound("/x".into()).to_string(), "not found: /x");
    }

    #[test]
    fn a_path_that_looks_like_an_option_becomes_absolute() {
        let machine = Machine::default().with("/work/repo/-rf/x", PathKind::File);
        assert_eq!(
            open("-rf/x", "path", &machine),
            edit("/work/repo/-rf/x", None)
        );
    }

    #[test]
    fn a_relative_path_without_a_working_directory_is_not_found() {
        let request = OpenRequest {
            text: "-rf/x",
            pattern: "path",
            url_template: None,
            cwd: None,
            home: None,
            row: 0,
            screen: &[],
            search_roots: &[],
        };
        assert_eq!(
            resolve(&request, &Machine::default()),
            OpenTarget::Refused(Refusal::NotFound("-rf/x".into()))
        );
    }

    #[test]
    fn scripts_and_executables_open_in_the_editor_since_it_runs_nothing() {
        let machine = Machine::default()
            .with("/work/repo/run", PathKind::Executable)
            .with("/work/repo/setup.sh", PathKind::File)
            .with("/work/repo/build.command", PathKind::Executable)
            .with("/work/repo/x.tool", PathKind::File);
        for name in ["run", "setup.sh", "build.command", "x.tool"] {
            assert_eq!(
                open(&format!("./{name}"), "path", &machine),
                edit(&format!("/work/repo/{name}"), None),
                "{name}"
            );
        }
        assert_eq!(
            open("/work/repo/run:3", "path", &machine),
            edit("/work/repo/run", Some(3))
        );
    }

    #[test]
    fn bundles_and_installers_are_revealed_never_launched() {
        let machine = Machine::default()
            .with("/work/repo/Calc.app", PathKind::Directory)
            .with("/work/repo/Installer.PKG", PathKind::File)
            .with("/work/repo/Disk.dmg", PathKind::File)
            .with("/work/repo/Driver.mpkg", PathKind::Directory)
            .with("/work/repo/Foo.prefPane", PathKind::Directory)
            .with("/work/repo/Run.workflow", PathKind::Directory)
            .with("/work/repo/odd.command", PathKind::Directory)
            .with("/work/repo/src", PathKind::Directory);
        for name in [
            "Calc.app",
            "Installer.PKG",
            "Disk.dmg",
            "Driver.mpkg",
            "Foo.prefPane",
            "Run.workflow",
            "odd.command",
        ] {
            assert_eq!(
                open(&format!("./{name}"), "path", &machine),
                OpenTarget::Reveal(format!("/work/repo/{name}").into()),
                "{name}"
            );
        }
        assert_eq!(
            open("./src", "path", &machine),
            OpenTarget::Directory("/work/repo/src".into())
        );
    }

    #[test]
    fn a_file_linking_to_an_installer_is_revealed() {
        let machine = Machine::default()
            .with("/work/repo/notes.txt", PathKind::File)
            .linking("/work/repo/notes.txt", "/tmp/Setup.pkg");
        assert_eq!(
            open("notes.txt", "path", &machine),
            OpenTarget::Reveal("/work/repo/notes.txt".into())
        );
    }

    #[test]
    fn a_relative_path_is_found_in_the_working_directory_first() {
        let machine = Machine::default()
            .with_repository("/work/repo")
            .with("/work/repo/sub/src/x.rs", PathKind::File)
            .with("/work/repo/src/x.rs", PathKind::File)
            .with_repository("/elsewhere")
            .with("/elsewhere/src/x.rs", PathKind::File);
        let ask = Ask::new("src/x.rs", "path")
            .from("/work/repo/sub")
            .beside("/elsewhere", 0);
        assert_eq!(ask.resolve(&machine), edit("/work/repo/sub/src/x.rs", None));
    }

    #[test]
    fn a_relative_path_is_found_at_the_git_root_of_the_working_directory() {
        let machine = Machine::default()
            .with_repository("/work/repo")
            .with("/work/repo/src/x.rs", PathKind::File)
            .with("/work/repo/sub", PathKind::Directory);
        assert_eq!(
            Ask::new("src/x.rs:9", "path")
                .from("/work/repo/sub")
                .resolve(&machine),
            edit("/work/repo/src/x.rs", Some(9))
        );
    }

    #[test]
    fn a_relative_path_is_found_in_the_nearest_folder_the_screen_names() {
        let machine = Machine::default()
            .with("/a", PathKind::Directory)
            .with("/a/src/x.rs", PathKind::File)
            .with("/b", PathKind::Directory)
            .with("/b/src/x.rs", PathKind::File)
            .with("/c", PathKind::Directory)
            .with("/c/src/x.rs", PathKind::File);
        let ask = |rows: [usize; 3]| {
            Ask::new("src/x.rs", "path")
                .from("/nowhere")
                .at_row(5)
                .beside("/a", rows[0])
                .beside("/b", rows[1])
                .beside("/c", rows[2])
                .resolve(&machine)
        };
        assert_eq!(ask([1, 5, 9]), edit("/b/src/x.rs", None), "same row");
        assert_eq!(ask([1, 3, 6]), edit("/b/src/x.rs", None), "rows above");
        assert_eq!(ask([9, 7, 6]), edit("/c/src/x.rs", None), "rows below");
    }

    #[test]
    fn a_file_on_screen_lends_its_git_root() {
        let machine = Machine::default()
            .with_repository("/home/me/dev/fingers")
            .with("/home/me/dev/fingers/README.md", PathKind::File)
            .with("/home/me/dev/fingers/src/domain/open.rs", PathKind::File);
        assert_eq!(
            Ask::new("src/domain/open.rs", "path")
                .from("/home/me/dev/notes")
                .beside("~/dev/fingers/README.md:4", 0)
                .resolve(&machine),
            edit("/home/me/dev/fingers/src/domain/open.rs", None)
        );
    }

    #[test]
    fn a_relative_path_in_exactly_one_search_root_is_found() {
        let machine = Machine::default()
            .with("/home/me/dev", PathKind::Directory)
            .with("/home/me/dev/notes", PathKind::Directory)
            .with("/home/me/dev/fingers", PathKind::Directory)
            .with("/home/me/dev/fingers/src/domain/open.rs", PathKind::File);
        assert_eq!(
            Ask::new("src/domain/open.rs", "path")
                .from("/home/me/dev/notes")
                .searching(&["~/dev/*"])
                .resolve(&machine),
            edit("/home/me/dev/fingers/src/domain/open.rs", None)
        );
    }

    #[test]
    fn a_relative_path_in_several_search_roots_is_refused() {
        let machine = Machine::default()
            .with("/home/me/dev/a/src/lib.rs", PathKind::File)
            .with("/home/me/dev/b/src/lib.rs", PathKind::File)
            .with("/home/me/dev/c/src/lib.rs", PathKind::File)
            .with("/home/me/dev/a", PathKind::Directory)
            .with("/home/me/dev/b", PathKind::Directory)
            .with("/home/me/dev/c", PathKind::Directory)
            .with("/srv/one/src/lib.rs", PathKind::File);
        let refusal = Ask::new("src/lib.rs", "path")
            .from("/nowhere")
            .searching(&["~/dev/*", "/srv/one"])
            .resolve(&machine);
        assert_eq!(
            refusal,
            OpenTarget::Refused(Refusal::Ambiguous {
                path: "src/lib.rs".into(),
                places: 4
            })
        );
        let OpenTarget::Refused(refusal) = refusal else {
            unreachable!()
        };
        assert_eq!(
            refusal.to_string(),
            "found in 4 places; not guessing which: src/lib.rs"
        );
    }

    #[test]
    fn a_relative_path_is_not_taken_as_unique_when_the_search_roots_have_too_many_folders() {
        let mut machine = Machine::default();
        for project in 0..=MAX_SEARCH_DIRS {
            machine = machine.with(&format!("/home/me/dev/p{project:04}"), PathKind::Directory);
        }
        let last = format!("/home/me/dev/p{MAX_SEARCH_DIRS:04}/src/x.rs");
        let machine = machine
            .with("/home/me/dev/p0000/src/x.rs", PathKind::File)
            .with(&last, PathKind::File);
        assert_eq!(
            Ask::new("src/x.rs", "path")
                .from("/nowhere")
                .searching(&["~/dev/*"])
                .resolve(&machine),
            OpenTarget::Refused(Refusal::TooManyFolders("src/x.rs".into()))
        );
    }

    #[test]
    fn a_relative_path_found_nowhere_is_reported_against_the_working_directory() {
        let machine = Machine::default()
            .with("/home/me/dev/a", PathKind::Directory)
            .with("/x", PathKind::Directory);
        assert_eq!(
            Ask::new("src/gone.rs", "path")
                .beside("/x", 0)
                .searching(&["~/dev/*"])
                .resolve(&machine),
            OpenTarget::Refused(Refusal::NotFound("/work/repo/src/gone.rs".into()))
        );
    }

    #[test]
    fn a_folder_on_screen_never_changes_what_kind_of_action_runs() {
        let machine = Machine::default()
            .with("/evil", PathKind::Directory)
            .with("/evil/docs", PathKind::Directory)
            .linking("/evil/docs", "/Applications/Calc.app");
        assert_eq!(
            Ask::new("docs", "path")
                .beside("/evil", 0)
                .resolve(&machine),
            OpenTarget::Reveal("/evil/docs".into())
        );
    }

    #[test]
    fn a_bare_file_name_opens_when_it_exists() {
        let machine = Machine::default()
            .with("/work/repo/README.md", PathKind::File)
            .with("/work/repo/src", PathKind::Directory);
        assert_eq!(
            open("README.md", "custom", &machine),
            edit("/work/repo/README.md", None)
        );
        assert_eq!(
            open("src", "custom", &machine),
            OpenTarget::Directory("/work/repo/src".into())
        );
        assert_eq!(
            open("CHANGELOG.md", "custom", &machine),
            OpenTarget::Refused(Refusal::NothingToOpen)
        );
    }

    #[test]
    fn a_number_or_id_never_opens_a_file_of_the_same_name() {
        let machine = Machine::default()
            .with("/work/repo/20260727", PathKind::Directory)
            .with("/work/repo/10.0.0.1", PathKind::File);
        for (text, pattern) in [
            ("20260727", "digit"),
            ("10.0.0.1", "ip"),
            ("20260727", SHA_PATTERN),
        ] {
            assert_eq!(
                open(text, pattern, &machine),
                OpenTarget::Refused(Refusal::NothingToOpen),
                "{pattern}"
            );
        }
    }

    #[test]
    fn a_bare_file_name_is_searched_for_like_any_relative_path() {
        let machine = Machine::default()
            .with("/home/me/dev/fingers", PathKind::Directory)
            .with("/home/me/dev/fingers/Cargo.toml", PathKind::File);
        assert_eq!(
            Ask::new("Cargo.toml", "custom")
                .from("/nowhere")
                .searching(&["~/dev/*"])
                .resolve(&machine),
            edit("/home/me/dev/fingers/Cargo.toml", None)
        );
    }

    #[test]
    fn a_sha_outside_a_github_repository_opens_the_nearest_github_link_on_screen() {
        let machine = Machine::default();
        assert_eq!(
            Ask::new("8b1a995", SHA_PATTERN)
                .at_row(4)
                .beside("https://github.com/far/away", 0)
                .beside("https://example.com/x/y", 4)
                .beside("https://github.com/o/r/pull/12?x=1", 3)
                .resolve(&machine),
            OpenTarget::Url("https://github.com/o/r/commit/8b1a995".into())
        );
    }

    #[test]
    fn a_sha_outside_a_github_repository_uses_a_repository_named_on_screen() {
        let machine = Machine::default()
            .with_repository("/home/me/dev/fingers")
            .with("/home/me/dev/fingers/src", PathKind::Directory)
            .with_origin_at("/home/me/dev/fingers", "git@github.com:me/fingers.git");
        assert_eq!(
            Ask::new("8b1a995", SHA_PATTERN)
                .from("/home/me/dev/notes")
                .beside("~/dev/fingers/src", 1)
                .resolve(&machine),
            OpenTarget::Url("https://github.com/me/fingers/commit/8b1a995".into())
        );
    }

    #[test]
    fn a_github_page_that_is_not_a_repository_is_no_clue_for_a_sha() {
        assert_eq!(
            Ask::new("8b1a995", SHA_PATTERN)
                .at_row(4)
                .beside("https://github.com/o/r", 0)
                .beside("https://github.com/settings/tokens", 3)
                .beside("https://github.com/orgs/acme/teams", 4)
                .resolve(&Machine::default()),
            OpenTarget::Url("https://github.com/o/r/commit/8b1a995".into())
        );
    }

    #[test]
    fn a_sha_asks_git_once_per_repository_on_screen_and_never_outside_one() {
        let machine = Machine::default()
            .with_repository("/home/me/dev/fingers")
            .with("/home/me/dev/fingers/src", PathKind::Directory)
            .with("/home/me/dev/fingers/README.md", PathKind::File)
            .with("/tmp/plain", PathKind::Directory)
            .with_origin_at("/home/me/dev/fingers", "git@gitlab.com:me/fingers.git");
        let ask = Ask::new("8b1a995", SHA_PATTERN)
            .from("/home/me/dev/notes")
            .beside("~/dev/fingers/src", 1)
            .beside("~/dev/fingers/README.md", 2)
            .beside("~/dev/fingers/src", 3)
            .beside("/tmp/plain", 4);
        assert_eq!(ask.resolve(&machine), OpenTarget::CopySha("8b1a995".into()));
        assert_eq!(machine.git_runs.get(), 2, "the pane, then fingers once");
    }

    #[test]
    fn the_pane_repository_wins_over_the_screen_for_a_sha() {
        let machine = Machine::default().with_origin("git@github.com:o/r.git");
        assert_eq!(
            Ask::new("8b1a995", SHA_PATTERN)
                .beside("https://github.com/x/y", 0)
                .resolve(&machine),
            OpenTarget::Url("https://github.com/o/r/commit/8b1a995".into())
        );
    }

    #[test]
    fn search_roots_are_absolute_with_at_most_a_final_star() {
        assert!(SearchRoot::parse("~/dev/*").is_ok());
        assert!(SearchRoot::parse("/srv/code/").is_ok());
        assert_eq!(
            SearchRoot::parse("/*"),
            Ok(SearchRoot {
                dir: "/".into(),
                each_subdirectory: true
            })
        );
        assert_eq!(
            SearchRoot::default_roots(),
            vec![SearchRoot::parse("~/dev/*").unwrap()]
        );
        for wild in ["~/dev/**", "~/*/src", "/srv/pro?", "~/dev/x*", "*"] {
            assert!(
                matches!(SearchRoot::parse(wild), Err(SearchRootError::Wildcard(_))),
                "{wild}"
            );
        }
        assert!(matches!(
            SearchRoot::parse("dev/*"),
            Err(SearchRootError::NotAbsolute(_))
        ));
    }

    #[test]
    fn a_link_to_an_app_bundle_is_revealed_whatever_its_own_name() {
        let machine = Machine::default()
            .with("/work/repo/docs", PathKind::Directory)
            .linking("/work/repo/docs", "/Applications/Calc.app");
        assert_eq!(
            open("./docs", "path", &machine),
            OpenTarget::Reveal("/work/repo/docs".into())
        );
    }

    #[test]
    fn a_file_name_with_spaces_quotes_and_dollars_is_kept_whole() {
        let name = "/work/repo/a b'c\"$(touch x).txt";
        let machine = Machine::default().with(name, PathKind::File);
        assert_eq!(open(name, "git-status", &machine), edit(name, None));
    }

    #[test]
    fn text_that_is_neither_link_nor_path_opens_nothing() {
        let machine = Machine::default();
        assert_eq!(
            open("48213", "digit", &machine),
            OpenTarget::Refused(Refusal::NothingToOpen)
        );
        assert_eq!(Refusal::NothingToOpen.to_string(), "nothing to open");
    }

    #[test]
    fn the_editor_command_places_the_path_and_line() {
        let words = |list: &[&str]| list.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let plain = EditorCommand::new("nvim", words(&["nvim"])).unwrap();
        assert_eq!(plain.argv("/a b", Some(3)), words(&["nvim", "+3", "/a b"]));
        assert_eq!(plain.argv("/a b", None), words(&["nvim", "/a b"]));
        let custom =
            EditorCommand::new("code --goto {path}", words(&["code", "--goto", "{path}"])).unwrap();
        assert_eq!(custom.argv("/x", Some(3)), words(&["code", "--goto", "/x"]));
        let hx = EditorCommand::new("hx --line={line}", words(&["hx", "--line={line}"])).unwrap();
        assert_eq!(hx.argv("/x", Some(7)), words(&["hx", "--line=7", "/x"]));
        assert_eq!(EditorCommand::new("", Vec::new()), Err(EditorError::Empty));
        assert!(matches!(
            EditorCommand::new("", words(&["code", "-g", "{path}:{line}"])),
            Err(EditorError::PathInsideWord(_))
        ));
    }
}
