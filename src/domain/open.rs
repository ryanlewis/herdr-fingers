//! What `:open:` does with a picked text: which URL, folder or file it
//! stands for, and how that is safe to open.

use std::fmt;
use std::path::{Path, PathBuf};

use regex::Regex;
use thiserror::Error;

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
    NothingToOpen,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Scheme(scheme) => write!(f, "won't open {scheme} links"),
            Refusal::NotFound(path) => write!(f, "not found: {}", path.display()),
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
    /// Something that would run if opened: show it in its folder instead.
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

/// Opening these would run something, so they are only ever revealed.
const LAUNCHABLE_EXTENSIONS: &[&str] = &[
    "app", "command", "terminal", "tool", "workflow", "pkg", "dmg", "sh",
];

/// Everything about one picked text the resolver needs.
pub struct OpenRequest<'a> {
    pub text: &'a str,
    pub pattern: &'a str,
    pub url_template: Option<&'a UrlTemplate>,
    /// The source pane's working directory.
    pub cwd: Option<&'a Path>,
    pub home: Option<&'a Path>,
}

/// What the resolver asks of the machine it runs on.
pub trait Surroundings {
    fn kind(&self, path: &Path) -> PathKind;
    /// Where `path` leads once every link in it is followed.
    fn real_path(&self, path: &Path) -> PathBuf;
    /// The `origin` remote URL of the Git repository containing `dir`.
    fn git_origin(&self, dir: &Path) -> Option<String>;
}

pub fn resolve<S: Surroundings + ?Sized>(
    request: &OpenRequest<'_>,
    surroundings: &S,
) -> OpenTarget {
    let text = request.text;
    if let Some(template) = request.url_template {
        return OpenTarget::Url(template.expand(text));
    }
    if let Some(scheme) = link_scheme(text) {
        return if scheme == "http" || scheme == "https" {
            OpenTarget::Url(text.to_string())
        } else {
            OpenTarget::Refused(Refusal::Scheme(scheme))
        };
    }
    if request.pattern == SHA_PATTERN {
        return request
            .cwd
            .and_then(|cwd| surroundings.git_origin(cwd))
            .and_then(|origin| github_repository(&origin))
            .map_or_else(
                || OpenTarget::CopySha(text.to_string()),
                |repository| OpenTarget::Url(format!("{repository}/commit/{text}")),
            );
    }
    if PATH_PATTERNS.contains(&request.pattern) || text.contains('/') || text.starts_with('~') {
        return resolve_path(request, surroundings);
    }
    OpenTarget::Refused(Refusal::NothingToOpen)
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

fn resolve_path<S: Surroundings + ?Sized>(
    request: &OpenRequest<'_>,
    surroundings: &S,
) -> OpenTarget {
    let (written, line) = split_line_suffix(request.text);
    let Some(path) = absolute(written, request.cwd, request.home) else {
        return OpenTarget::Refused(Refusal::NotFound(PathBuf::from(written)));
    };
    let mut found = (path.clone(), surroundings.kind(&path));
    if found.1 == PathKind::Missing
        && let Some(unprefixed) = written
            .strip_prefix("a/")
            .or_else(|| written.strip_prefix("b/"))
        && let Some(retry) = absolute(unprefixed, request.cwd, request.home)
    {
        let kind = surroundings.kind(&retry);
        if kind != PathKind::Missing {
            found = (retry, kind);
        }
    }
    let (path, kind) = found;
    // The opener follows links, so a link named `docs` to an app bundle
    // would launch it: what the path leads to counts too.
    let launchable = kind != PathKind::Missing
        && (is_launchable(&path) || is_launchable(&surroundings.real_path(&path)));
    match kind {
        PathKind::Missing => OpenTarget::Refused(Refusal::NotFound(path)),
        PathKind::Executable => OpenTarget::Reveal(path),
        _ if launchable => OpenTarget::Reveal(path),
        PathKind::Directory => OpenTarget::Directory(path),
        PathKind::File => OpenTarget::Edit(FileLocation { path, line }),
    }
}

fn is_launchable(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            LAUNCHABLE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
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
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Machine {
        kinds: BTreeMap<PathBuf, PathKind>,
        links: BTreeMap<PathBuf, PathBuf>,
        origin: Option<String>,
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

        fn with_origin(mut self, origin: &str) -> Self {
            self.origin = Some(origin.to_string());
            self
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

        fn git_origin(&self, _dir: &Path) -> Option<String> {
            self.origin.clone()
        }
    }

    fn open(text: &str, pattern: &str, machine: &Machine) -> OpenTarget {
        resolve(
            &OpenRequest {
                text,
                pattern,
                url_template: None,
                cwd: Some(Path::new("/work/repo")),
                home: Some(Path::new("/home/me")),
            },
            machine,
        )
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
        };
        assert_eq!(
            resolve(&request, &Machine::default()),
            OpenTarget::Refused(Refusal::NotFound("-rf/x".into()))
        );
    }

    #[test]
    fn executables_and_app_bundles_are_revealed_never_launched() {
        let machine = Machine::default()
            .with("/work/repo/run", PathKind::Executable)
            .with("/work/repo/Calc.app", PathKind::Directory)
            .with("/work/repo/setup.sh", PathKind::File)
            .with("/work/repo/Installer.PKG", PathKind::File)
            .with("/work/repo/src", PathKind::Directory);
        assert_eq!(
            open("./run", "path", &machine),
            OpenTarget::Reveal("/work/repo/run".into())
        );
        assert_eq!(
            open("/work/repo/run", "path", &machine),
            OpenTarget::Reveal("/work/repo/run".into())
        );
        assert_eq!(
            open("./Calc.app", "path", &machine),
            OpenTarget::Reveal("/work/repo/Calc.app".into())
        );
        assert_eq!(
            open("./setup.sh", "path", &machine),
            OpenTarget::Reveal("/work/repo/setup.sh".into())
        );
        assert_eq!(
            open("./Installer.PKG", "path", &machine),
            OpenTarget::Reveal("/work/repo/Installer.PKG".into())
        );
        assert_eq!(
            open("./src", "path", &machine),
            OpenTarget::Directory("/work/repo/src".into())
        );
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
