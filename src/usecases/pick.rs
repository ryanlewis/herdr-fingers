//! `pick`: read the pane, label the matches, let the user choose, act.

use std::collections::BTreeMap;
use std::path::Path;

use thiserror::Error;

use super::ports::{
    Clipboard, FileSystem, Launcher, PaneHost, PickView, Picker, PortError, SplitPane,
};
use crate::domain::geometry::OverlayGeometry;
use crate::domain::links::{self, LINK_PATTERN};
use crate::domain::matcher::{Candidate, Origin, find_candidates};
use crate::domain::open::{self, FileLocation, OpenRequest, OpenTarget, ScreenMatch};
use crate::domain::screen::Screen;
use crate::domain::session::{Outcome, Selection, Session};
use crate::domain::settings::{Action, Settings};

/// The pane entrypoint declared in `herdr-plugin.toml` for the editor.
pub const EDITOR_ENTRYPOINT: &str = "editor";
/// Environment variables carrying the editor command line, the file and the
/// line from `pick` to `edit`.
pub const EDITOR_ENV: &str = "HERDR_FINGERS_EDITOR";
pub const EDIT_PATH_ENV: &str = "HERDR_FINGERS_EDIT_PATH";
pub const EDIT_LINE_ENV: &str = "HERDR_FINGERS_EDIT_LINE";

/// Where the overlay's hints come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintSource {
    /// Matches of the configured patterns.
    Patterns,
    /// Links the host reports under coloured or underlined text.
    Links,
}

/// Link lookups per overlay. Each may run a host link handler, and a busy
/// screen has hundreds of coloured runs.
pub const MAX_LINK_LOOKUPS: usize = 64;

/// What one run of the overlay works on.
pub struct PickRequest<'a> {
    /// This plugin's id, to open its editor pane.
    pub plugin_id: &'a str,
    pub pane_id: &'a str,
    /// Where the pane sits in the overlay; `None` when launched by hand.
    pub geometry: Option<&'a OverlayGeometry>,
    pub settings: &'a Settings,
    pub source: HintSource,
    /// A one-line notice to show (for instance a configuration problem).
    pub notice: Option<&'a str>,
}

/// The adapters one run of the overlay goes through.
pub struct Deps<'a> {
    pub host: &'a dyn PaneHost,
    pub clipboard: &'a mut dyn Clipboard,
    pub launcher: &'a dyn Launcher,
    pub files: &'a dyn FileSystem,
    pub picker: &'a mut dyn Picker,
}

#[derive(Debug, Error)]
pub enum PickError {
    #[error(transparent)]
    Port(#[from] PortError),
}

/// How the run ended, with a line for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Cancelled,
    Done(String),
}

pub fn pick(request: &PickRequest<'_>, deps: &mut Deps<'_>) -> Result<Picked, PickError> {
    let width = match request.geometry {
        Some(geometry) => geometry.pane.width,
        None => deps
            .host
            .layout(request.pane_id)
            .ok()
            .and_then(|layout| {
                layout
                    .panes
                    .into_iter()
                    .find(|p| p.pane_id == request.pane_id)
            })
            .map_or(0, |p| p.rect.width),
    };
    let screen = Screen::from_ansi(&deps.host.read_visible(request.pane_id)?, width);
    let (candidates, source_notice) = match request.source {
        HintSource::Patterns => (find_candidates(&screen, &request.settings.patterns), None),
        HintSource::Links => match find_links(&screen, request.pane_id, deps.host) {
            Ok(found) if !found.is_empty() => (found, None),
            Ok(_) => (
                find_candidates(&screen, &request.settings.patterns),
                Some("no links found; showing the usual hints".to_string()),
            ),
            Err(error) => (
                find_candidates(&screen, &request.settings.patterns),
                Some(format!(
                    "can't look up links ({error}); showing the usual hints"
                )),
            ),
        },
    };
    let on_screen: Vec<ScreenMatch> = candidates
        .iter()
        .filter_map(|candidate| {
            candidate.segments.first().map(|segment| ScreenMatch {
                text: &candidate.text,
                row: segment.row,
            })
        })
        .collect();
    let fresh_session = || Session::new(candidates.clone(), &request.settings.alphabet);
    let mut session = fresh_session();
    let mut notice = match (request.notice, source_notice) {
        (Some(first), Some(second)) => Some(format!("{first} · {second}")),
        (first, second) => first.map(str::to_string).or(second),
    };
    loop {
        let view = PickView {
            screen: &screen,
            theme: &request.settings.theme,
            geometry: request.geometry,
            notice: notice.as_deref(),
            actions: &request.settings.actions,
            show_legend: request.settings.show_legend,
        };
        match deps.picker.pick(&view, &mut session)? {
            Outcome::Picked(selection) => {
                let cwd = deps.host.pane_cwd(request.pane_id).unwrap_or(None);
                let context = Context {
                    cwd: cwd.as_deref(),
                    on_screen: &on_screen,
                };
                match dispatch(&selection, request, &context, deps)? {
                    Dispatched::Done(message) => return Ok(Picked::Done(message)),
                    // Nothing happened: keep the hints up and say why, and
                    // start over so another key (Ctrl to copy, say) acts on
                    // one hint rather than the refused multi-selection.
                    Dispatched::Refused(reason) => {
                        notice = Some(reason);
                        session = fresh_session();
                    }
                }
            }
            Outcome::Cancelled | Outcome::Continue => return Ok(Picked::Cancelled),
        }
    }
}

/// The links under the styled runs nearest the bottom of the screen, in
/// reading order, each asked about once at its first cell.
fn find_links(
    screen: &Screen,
    pane_id: &str,
    host: &dyn PaneHost,
) -> Result<Vec<Candidate>, PortError> {
    let mut found = Vec::new();
    for run in links::styled_runs(screen)
        .into_iter()
        .rev()
        .take(MAX_LINK_LOOKUPS)
    {
        // A first failure means the host can't answer at all; a later one
        // (the pane redrew mid-scan) should not cost the links already found.
        let url = match host.link_at(pane_id, run.row, run.col) {
            Ok(url) => url,
            Err(_) if !found.is_empty() => break,
            Err(error) => return Err(error),
        };
        if let Some(url) = url {
            found.push(Candidate {
                text: url,
                pattern: LINK_PATTERN.to_string(),
                origin: Origin::HostLink,
                segments: vec![run],
            });
        }
    }
    found.reverse();
    Ok(found)
}

enum Dispatched {
    /// Done, with a line for the log.
    Done(String),
    /// Nothing was done, for this reason.
    Refused(String),
}

/// Where the pick was made: the pane's directory and the rest of its screen.
struct Context<'a> {
    cwd: Option<&'a Path>,
    on_screen: &'a [ScreenMatch<'a>],
}

/// Carries out the action bound to the modifier the user held.
fn dispatch(
    selection: &Selection,
    request: &PickRequest<'_>,
    context: &Context<'_>,
    deps: &mut Deps<'_>,
) -> Result<Dispatched, PickError> {
    let settings = request.settings;
    let cwd = context.cwd;
    let text = selection.texts().join(&settings.multi_separator);
    match settings.actions.for_modifier(selection.modifier) {
        Action::Copy => {
            deps.clipboard.copy(&text)?;
            if settings.notify_on_copy
                && let Err(error) = deps.host.notify("Copied", &preview(&text))
            {
                return Ok(Dispatched::Done(format!(
                    "{} (toast failed: {error})",
                    record("copied", selection, &text, settings)
                )));
            }
            Ok(Dispatched::Done(record(
                "copied", selection, &text, settings,
            )))
        }
        Action::Paste => {
            deps.host.send_text(request.pane_id, &text)?;
            Ok(Dispatched::Done(record(
                "pasted", selection, &text, settings,
            )))
        }
        Action::Shell(command_line) => {
            let env = [
                ("MODIFIER", selection.modifier.as_str().to_string()),
                ("HINT", selection.hint.clone()),
            ];
            deps.launcher.run(command_line, &text, &env, cwd)?;
            Ok(Dispatched::Done(format!("ran {command_line}")))
        }
        Action::Open => open_all(selection, &text, request, context, deps),
        Action::Nothing => Ok(Dispatched::Done(String::new())),
    }
}

/// The log line for a finished action. The picked text only goes in when the
/// user asked for it, since it can be a token or a password.
fn record(verb: &str, selection: &Selection, text: &str, settings: &Settings) -> String {
    if settings.log_picked_text {
        return format!("{verb} {}", preview(text));
    }
    let count = selection.picks.len();
    let noun = if count == 1 { "item" } else { "items" };
    format!("{verb} {count} {noun} ({} chars)", text.chars().count())
}

/// Opens every pick, or none of them when one cannot be opened.
fn open_all(
    selection: &Selection,
    text: &str,
    request: &PickRequest<'_>,
    context: &Context<'_>,
    deps: &mut Deps<'_>,
) -> Result<Dispatched, PickError> {
    let cwd = context.cwd;
    let home = deps.files.home_dir();
    let targets: Vec<OpenTarget> = selection
        .picks
        .iter()
        .map(|pick| {
            let request = OpenRequest {
                text: &pick.text,
                pattern: &pick.pattern,
                origin: pick.origin,
                url_template: request.settings.patterns.open_url(&pick.pattern),
                cwd,
                home: home.as_deref(),
                row: pick.row,
                screen: context.on_screen,
                search_roots: &request.settings.search_roots,
            };
            open::resolve(&request, deps.files)
        })
        .collect();
    if let Some(refusal) = targets.iter().find_map(|target| match target {
        OpenTarget::Refused(refusal) => Some(refusal),
        _ => None,
    }) {
        return Ok(Dispatched::Refused(refusal.to_string()));
    }
    let mut verbs = Vec::new();
    for target in targets {
        verbs.push(open_target(target, request, cwd, deps)?);
    }
    let verb = match verbs.as_slice() {
        [only] => only,
        _ => "opened",
    };
    Ok(Dispatched::Done(record(
        verb,
        selection,
        text,
        request.settings,
    )))
}

/// Carries out one resolved target; returns what was done, for the log.
fn open_target(
    target: OpenTarget,
    request: &PickRequest<'_>,
    cwd: Option<&Path>,
    deps: &mut Deps<'_>,
) -> Result<&'static str, PickError> {
    match target {
        OpenTarget::Url(url) => {
            deps.launcher.open(&url, cwd)?;
            Ok("opened")
        }
        OpenTarget::Directory(path) => {
            deps.launcher.open(&path.to_string_lossy(), cwd)?;
            Ok("opened")
        }
        OpenTarget::Reveal(path) => {
            deps.launcher.reveal(&path)?;
            Ok("revealed")
        }
        OpenTarget::Edit(location) => edit(location, request, cwd, deps),
        OpenTarget::CopySha(sha) => {
            deps.clipboard.copy(&sha)?;
            let _ = deps
                .host
                .notify("Copied", "No GitHub remote here, so the SHA was copied");
            Ok("copied")
        }
        OpenTarget::Refused(_) => Ok("refused"),
    }
}

/// Opens the file in the editor beside the source pane, or reveals it when
/// there is no editor to run.
fn edit(
    location: FileLocation,
    request: &PickRequest<'_>,
    cwd: Option<&Path>,
    deps: &mut Deps<'_>,
) -> Result<&'static str, PickError> {
    let Some(editor) = &request.settings.editor else {
        deps.launcher.reveal(&location.path)?;
        return Ok("revealed");
    };
    let mut env = BTreeMap::new();
    env.insert(EDITOR_ENV.to_string(), editor.template().to_string());
    env.insert(
        EDIT_PATH_ENV.to_string(),
        location.path.to_string_lossy().into_owned(),
    );
    if let Some(line) = location.line {
        env.insert(EDIT_LINE_ENV.to_string(), line.to_string());
    }
    deps.host.open_split(&SplitPane {
        plugin_id: request.plugin_id,
        entrypoint: EDITOR_ENTRYPOINT,
        beside: request.pane_id,
        direction: request.settings.editor_split,
        cwd,
        env,
    })?;
    Ok("edited")
}

/// The first line of `text`, shortened for a status line or a toast.
pub fn preview(text: &str) -> String {
    const MAX_CHARS: usize = 40;
    let first_line = text.lines().next().unwrap_or("");
    let mut shortened: String = first_line.chars().take(MAX_CHARS).collect();
    if first_line.chars().count() > MAX_CHARS || text.lines().count() > 1 {
        shortened.push('…');
    }
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::alphabet::Alphabet;
    use crate::domain::geometry::Rect;
    use crate::domain::open::{EditorCommand, PathKind, UrlTemplate};
    use crate::domain::patterns::{PatternSet, PatternSpec, builtin_specs};
    use crate::domain::session::{Key, Modifier};
    use crate::domain::settings::SplitDirection;
    use crate::usecases::testing::{
        FakeFiles, FakeHost, OpenedSplit, RecordingClipboard, RecordingLauncher, ScriptedPicker,
    };
    use std::path::PathBuf;

    const SCREEN: &str = "edit /etc/hosts now\nsee https://x.io/docs\n";

    fn settings() -> Settings {
        Settings {
            alphabet: Alphabet::custom("asdf").unwrap(),
            ..Settings::default()
        }
    }

    fn geometry() -> OverlayGeometry {
        OverlayGeometry {
            pane_id: "w1:p1".into(),
            area: Rect::new(0, 0, 80, 24),
            pane: Rect::new(0, 0, 80, 24),
        }
    }

    struct World {
        host: FakeHost,
        clipboard: RecordingClipboard,
        launcher: RecordingLauncher,
        files: FakeFiles,
        picker: ScriptedPicker,
        source: HintSource,
    }

    impl World {
        fn new(keys: &[Key]) -> Self {
            World {
                source: HintSource::Patterns,
                host: FakeHost::showing(SCREEN),
                clipboard: RecordingClipboard::default(),
                launcher: RecordingLauncher::default(),
                files: FakeFiles::default(),
                picker: ScriptedPicker::new(keys),
            }
        }

        fn run(
            &mut self,
            settings: &Settings,
            geometry: Option<&OverlayGeometry>,
            notice: Option<&str>,
        ) -> Picked {
            let request = PickRequest {
                plugin_id: "test.fingers",
                pane_id: "w1:p1",
                geometry,
                settings,
                source: self.source,
                notice,
            };
            let mut deps = Deps {
                host: &self.host,
                clipboard: &mut self.clipboard,
                launcher: &self.launcher,
                files: &self.files,
                picker: &mut self.picker,
            };
            pick(&request, &mut deps).unwrap()
        }
    }

    // On SCREEN with alphabet "asdf": the bottom match (the URL) is "a",
    // the path above it is "s".

    #[test]
    fn ctrl_copies_the_text_and_touches_nothing_else() {
        let mut world = World::new(&[Key::Hint('s', Modifier::Ctrl)]);
        let picked = world.run(&settings(), Some(&geometry()), None);
        assert_eq!(picked, Picked::Done("copied 1 item (10 chars)".into()));
        assert_eq!(world.clipboard.copied, vec!["/etc/hosts"]);
        assert!(world.host.sent.borrow().is_empty());
        assert!(world.launcher.opened.borrow().is_empty());
        assert!(world.host.notified.borrow().is_empty());
    }

    #[test]
    fn the_log_line_shows_the_text_only_when_asked_to() {
        let mut settings = settings();
        settings.log_picked_text = true;
        let mut world = World::new(&[Key::Hint('s', Modifier::Ctrl)]);
        let picked = world.run(&settings, Some(&geometry()), None);
        assert_eq!(picked, Picked::Done("copied /etc/hosts".into()));
    }

    #[test]
    fn a_multi_select_log_line_counts_the_items() {
        let mut world = World::new(&[
            Key::Tab,
            Key::Hint('a', Modifier::Ctrl),
            Key::Hint('s', Modifier::Ctrl),
            Key::Enter,
        ]);
        let picked = world.run(&settings(), Some(&geometry()), None);
        assert_eq!(picked, Picked::Done("copied 2 items (28 chars)".into()));
    }

    #[test]
    fn shift_types_the_text_into_the_pane_the_hints_came_from() {
        let mut world = World::new(&[Key::Hint('s', Modifier::Shift)]);
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            *world.host.sent.borrow(),
            vec![("w1:p1".to_string(), "/etc/hosts".to_string())]
        );
        assert!(world.clipboard.copied.is_empty());
    }

    #[test]
    fn a_plain_hint_opens_a_link_from_the_pane_working_directory() {
        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = world.host.in_directory("/work/repo");
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            *world.launcher.opened.borrow(),
            vec![("https://x.io/docs".to_string(), Some("/work/repo".into()))]
        );
    }

    #[test]
    fn alt_runs_the_shell_action_with_the_text_the_hint_and_the_modifier() {
        let mut settings = settings();
        settings.actions.alt = Action::Shell("xargs nvim".into());
        let mut world = World::new(&[Key::Hint('s', Modifier::Alt)]);
        let picked = world.run(&settings, Some(&geometry()), None);
        assert_eq!(picked, Picked::Done("ran xargs nvim".into()));
        let ran = world.launcher.ran.borrow();
        assert_eq!(ran[0].command_line, "xargs nvim");
        assert_eq!(ran[0].stdin, "/etc/hosts");
        assert_eq!(
            ran[0].env,
            vec![
                ("MODIFIER".into(), "alt".into()),
                ("HINT".into(), "s".into())
            ]
        );
    }

    #[test]
    fn cancelling_copies_nothing() {
        let mut world = World::new(&[Key::Hint('s', Modifier::Main), Key::Escape]);
        world.picker = ScriptedPicker::new(&[Key::Escape]);
        assert_eq!(
            world.run(&settings(), Some(&geometry()), None),
            Picked::Cancelled
        );
        assert!(world.clipboard.copied.is_empty());
        assert!(world.host.sent.borrow().is_empty());
    }

    #[test]
    fn multi_select_joins_the_picks_with_the_separator() {
        let mut settings = settings();
        settings.multi_separator = "\n".into();
        let mut world = World::new(&[
            Key::Tab,
            Key::Hint('a', Modifier::Ctrl),
            Key::Hint('s', Modifier::Ctrl),
            Key::Enter,
        ]);
        world.run(&settings, Some(&geometry()), None);
        assert_eq!(
            world.clipboard.copied,
            vec!["https://x.io/docs\n/etc/hosts"]
        );
    }

    #[test]
    fn a_copy_toast_goes_through_the_host_when_enabled() {
        let mut settings = settings();
        settings.notify_on_copy = true;
        let mut world = World::new(&[Key::Hint('a', Modifier::Ctrl)]);
        world.run(&settings, Some(&geometry()), None);
        assert_eq!(
            *world.host.notified.borrow(),
            vec![("Copied".to_string(), "https://x.io/docs".to_string())]
        );
    }

    #[test]
    fn the_notice_and_geometry_reach_the_picker() {
        let mut world = World::new(&[Key::Escape]);
        world.run(&settings(), Some(&geometry()), Some("config.toml ignored"));
        assert_eq!(world.picker.seen_notice, Some("config.toml ignored".into()));
        assert_eq!(world.picker.seen_geometry, Some(geometry()));
        assert!(world.host.layout_calls.borrow().is_empty());
    }

    #[test]
    fn the_configured_actions_and_the_legend_switch_reach_the_picker() {
        let mut settings = settings();
        settings.actions.alt = Action::Shell("xargs nvim".into());
        let mut world = World::new(&[Key::Escape]);
        world.run(&settings, Some(&geometry()), None);
        assert_eq!(
            world.picker.seen_legend,
            Some((settings.actions.clone(), true))
        );

        settings.show_legend = false;
        let mut world = World::new(&[Key::Escape]);
        world.run(&settings, Some(&geometry()), None);
        assert_eq!(
            world.picker.seen_legend,
            Some((settings.actions.clone(), false))
        );
    }

    #[test]
    fn without_geometry_the_pane_width_comes_from_the_host_layout() {
        let mut world = World::new(&[Key::Escape]);
        world.host = FakeHost::showing(SCREEN).with_layout(FakeHost::split_layout("w1:p1"));
        world.run(&settings(), None, None);
        assert_eq!(*world.host.layout_calls.borrow(), vec!["w1:p1".to_string()]);
    }

    // Two labelled links on the top row: "PR #42" in blue, "notes" underlined.
    const LINKS_SCREEN: &str =
        "see \x1b[34mPR #42\x1b[0m and \x1b[4mnotes\x1b[24m here\nplain https://x.io/docs\n";
    const PR_URL: &str = "https://github.com/o/r/pull/42";

    fn link_world(keys: &[Key]) -> World {
        let mut world = World::new(keys);
        world.source = HintSource::Links;
        world.host = FakeHost::showing(LINKS_SCREEN)
            .with_link(0, 4, 6, PR_URL)
            .with_link(0, 15, 5, "file:///work/notes.md");
        world
    }

    // In link mode on LINKS_SCREEN: "notes" is "a", "PR #42" is "s".

    #[test]
    fn link_mode_asks_about_each_styled_run_once_and_opens_the_link_picked() {
        let mut world = link_world(&[Key::Hint('s', Modifier::Main)]);
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(*world.host.link_calls.borrow(), vec![(0, 15), (0, 4)]);
        assert_eq!(
            *world.launcher.opened.borrow(),
            vec![(PR_URL.to_string(), None)]
        );
        assert_eq!(world.picker.seen_notice, None);
    }

    #[test]
    fn copying_a_link_copies_its_address_not_its_label() {
        let mut world = link_world(&[Key::Hint('s', Modifier::Ctrl)]);
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(world.clipboard.copied, vec![PR_URL]);
    }

    #[test]
    fn a_file_link_opens_as_the_file_it_names() {
        let mut world = link_world(&[Key::Hint('a', Modifier::Main)]);
        world.files = FakeFiles::default().with("/work/notes.md", PathKind::File);
        let picked = world.run(&settings(), Some(&geometry()), None);
        assert_eq!(picked, Picked::Done("revealed 1 item (21 chars)".into()));
        assert_eq!(
            *world.launcher.revealed.borrow(),
            vec![PathBuf::from("/work/notes.md")]
        );
    }

    #[test]
    fn without_link_lookups_the_usual_hints_come_back_with_a_notice() {
        let mut world = link_world(&[Key::Hint('a', Modifier::Ctrl)]);
        world.host = world.host.failing_links("pane is no longer visible");
        world.run(&settings(), Some(&geometry()), Some("config.toml ignored"));
        assert_eq!(world.host.link_calls.borrow().len(), 1);
        assert_eq!(
            world.picker.seen_notice.as_deref(),
            Some(
                "config.toml ignored · can't look up links (pane is no longer visible); \
                 showing the usual hints"
            )
        );
        assert_eq!(world.clipboard.copied, vec!["https://x.io/docs"]);
    }

    #[test]
    fn a_lookup_failing_after_links_were_found_keeps_those_links() {
        let mut world = link_world(&[Key::Hint('a', Modifier::Ctrl)]);
        world.host = world
            .host
            .failing_links_after(1, "pane content changed during link activation");
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(world.picker.seen_notice, None);
        assert_eq!(world.clipboard.copied, vec!["file:///work/notes.md"]);
    }

    #[test]
    fn a_screen_without_links_falls_back_to_the_usual_hints() {
        let mut world = World::new(&[Key::Hint('a', Modifier::Ctrl)]);
        world.source = HintSource::Links;
        world.host = FakeHost::showing(LINKS_SCREEN);
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            world.picker.seen_notice.as_deref(),
            Some("no links found; showing the usual hints")
        );
        assert_eq!(world.clipboard.copied, vec!["https://x.io/docs"]);
    }

    #[test]
    fn link_lookups_stop_at_the_cap_starting_from_the_bottom() {
        let screen: String = (0..MAX_LINK_LOOKUPS + 6)
            .map(|_| "\x1b[34mx\x1b[0m\n")
            .collect();
        let mut world = World::new(&[Key::Escape]);
        world.source = HintSource::Links;
        world.host = FakeHost::showing(&screen);
        world.run(&settings(), Some(&geometry()), None);
        let calls = world.host.link_calls.borrow();
        assert_eq!(calls.len(), MAX_LINK_LOOKUPS);
        assert_eq!(calls[0], (MAX_LINK_LOOKUPS + 5, 0));
    }

    #[test]
    fn a_host_that_cannot_read_the_pane_is_an_error() {
        let mut world = World::new(&[Key::Escape]);
        world.host = FakeHost::showing(SCREEN).failing_reads();
        let request = PickRequest {
            plugin_id: "test.fingers",
            pane_id: "w1:p1",
            geometry: None,
            settings: &settings(),
            source: HintSource::Patterns,
            notice: None,
        };
        let mut deps = Deps {
            host: &world.host,
            clipboard: &mut world.clipboard,
            launcher: &world.launcher,
            files: &world.files,
            picker: &mut world.picker,
        };
        assert!(matches!(pick(&request, &mut deps), Err(PickError::Port(_))));
    }

    fn editing_settings() -> Settings {
        Settings {
            editor: Some(EditorCommand::new("nvim", vec!["nvim".into()]).expect("valid editor")),
            editor_split: SplitDirection::Down,
            ..settings()
        }
    }

    #[test]
    fn a_file_opens_in_the_editor_split_beside_the_source_pane() {
        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = FakeHost::showing("see ./notes/a b.md:12:3 x\n").in_directory("/work");
        let mut settings = editing_settings();
        settings.patterns = PatternSet::compile(&[PatternSpec::new(
            "spaced",
            r"(?P<match>\./notes/a b\.md:12:3)",
        )])
        .unwrap();
        world.files = FakeFiles::default().with("/work/notes/a b.md", PathKind::File);
        let picked = world.run(&settings, Some(&geometry()), None);
        assert_eq!(picked, Picked::Done("edited 1 item (19 chars)".into()));
        let splits = world.host.splits.borrow();
        let mut env = BTreeMap::new();
        env.insert(EDITOR_ENV.to_string(), "nvim".to_string());
        env.insert(EDIT_PATH_ENV.to_string(), "/work/notes/a b.md".to_string());
        env.insert(EDIT_LINE_ENV.to_string(), "12".to_string());
        assert_eq!(
            *splits,
            vec![OpenedSplit {
                plugin_id: "test.fingers".into(),
                entrypoint: EDITOR_ENTRYPOINT.into(),
                beside: "w1:p1".into(),
                direction: SplitDirection::Down,
                cwd: Some("/work".into()),
                env,
            }]
        );
        assert!(world.launcher.opened.borrow().is_empty());
    }

    #[test]
    fn compiler_output_opens_the_file_at_its_line() {
        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = FakeHost::showing("error: src/x.rs:12:3: oops\n").in_directory("/work");
        world.files = FakeFiles::default().with("/work/src/x.rs", PathKind::File);
        world.run(&editing_settings(), Some(&geometry()), None);
        let splits = world.host.splits.borrow();
        assert_eq!(splits[0].env[EDIT_PATH_ENV], "/work/src/x.rs");
        assert_eq!(splits[0].env[EDIT_LINE_ENV], "12");
    }

    #[test]
    fn without_an_editor_a_file_is_revealed() {
        let mut world = World::new(&[Key::Hint('s', Modifier::Main)]);
        world.files = FakeFiles::default().with("/etc/hosts", PathKind::File);
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            *world.launcher.revealed.borrow(),
            vec![PathBuf::from("/etc/hosts")]
        );
        assert!(world.host.splits.borrow().is_empty());
    }

    #[test]
    fn an_executable_opens_in_the_editor_and_is_revealed_without_one() {
        let mut world = World::new(&[Key::Hint('s', Modifier::Main)]);
        world.files = FakeFiles::default().with("/etc/hosts", PathKind::Executable);
        world.run(&editing_settings(), Some(&geometry()), None);
        assert_eq!(
            world.host.splits.borrow()[0].env[EDIT_PATH_ENV],
            "/etc/hosts"
        );
        assert!(world.launcher.revealed.borrow().is_empty());

        let mut world = World::new(&[Key::Hint('s', Modifier::Main)]);
        world.files = FakeFiles::default().with("/etc/hosts", PathKind::Executable);
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            *world.launcher.revealed.borrow(),
            vec![PathBuf::from("/etc/hosts")]
        );
        assert!(world.host.splits.borrow().is_empty());
    }

    #[test]
    fn a_relative_path_is_found_in_a_folder_named_on_the_screen() {
        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = FakeHost::showing("cd /code/fingers\nsee src/lib.rs\n").in_directory("/notes");
        world.files = FakeFiles::default()
            .with("/code/fingers", PathKind::Directory)
            .with("/code/fingers/src/lib.rs", PathKind::File);
        world.run(&editing_settings(), Some(&geometry()), None);
        assert_eq!(
            world.host.splits.borrow()[0].env[EDIT_PATH_ENV],
            "/code/fingers/src/lib.rs"
        );
    }

    #[test]
    fn what_cannot_be_opened_keeps_the_hints_up_with_the_reason() {
        let mut world = World::new(&[
            Key::Hint('s', Modifier::Main),
            Key::Hint('s', Modifier::Ctrl),
        ]);
        let picked = world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            world.picker.seen_notice,
            Some("not found: /etc/hosts".into())
        );
        assert!(world.launcher.opened.borrow().is_empty());
        assert!(world.launcher.revealed.borrow().is_empty());
        assert_eq!(picked, Picked::Done("copied 1 item (10 chars)".into()));
    }

    #[test]
    fn after_a_refused_multi_select_ctrl_copies_a_single_hint() {
        let mut world = World::new(&[
            Key::Tab,
            Key::Hint('a', Modifier::Main),
            Key::Hint('s', Modifier::Main),
            Key::Enter,
            Key::Hint('a', Modifier::Ctrl),
        ]);
        let picked = world.run(&settings(), Some(&geometry()), None);
        assert_eq!(world.clipboard.copied, vec!["https://x.io/docs"]);
        assert!(world.launcher.opened.borrow().is_empty());
        assert_eq!(picked, Picked::Done("copied 1 item (17 chars)".into()));
    }

    #[test]
    fn a_link_with_another_scheme_is_refused() {
        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = FakeHost::showing("file:///Applications/Calculator.app\n");
        let picked = world.run(&settings(), Some(&geometry()), None);
        assert_eq!(picked, Picked::Cancelled);
        assert_eq!(
            world.picker.seen_notice,
            Some("won't open file links".into())
        );
        assert!(world.launcher.opened.borrow().is_empty());
        assert!(world.launcher.revealed.borrow().is_empty());
    }

    #[test]
    fn multi_select_opens_nothing_when_one_pick_cannot_be_opened() {
        let mut world = World::new(&[
            Key::Tab,
            Key::Hint('a', Modifier::Main),
            Key::Hint('s', Modifier::Main),
            Key::Enter,
        ]);
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            world.picker.seen_notice,
            Some("not found: /etc/hosts".into())
        );
        assert!(world.launcher.opened.borrow().is_empty());
    }

    #[test]
    fn a_ticket_key_opens_the_pattern_url() {
        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = FakeHost::showing("fixes RET-1234 today\n");
        let mut settings = settings();
        let mut specs = vec![
            PatternSpec::new("jira", r"\b(?P<match>[A-Z][A-Z0-9]+-[0-9]+)\b")
                .opening(UrlTemplate::parse("https://example.atlassian.net/browse/{}").unwrap()),
        ];
        specs.extend(builtin_specs(&["digit"]).unwrap());
        settings.patterns = PatternSet::compile(&specs).unwrap();
        world.run(&settings, Some(&geometry()), None);
        assert_eq!(
            *world.launcher.opened.borrow(),
            vec![(
                "https://example.atlassian.net/browse/RET-1234".to_string(),
                None
            )]
        );
    }

    #[test]
    fn a_sha_opens_its_github_commit_or_is_copied_elsewhere() {
        let screen = "commit 8b1a9953c4611299a820df69698463c3ca01599d\n";
        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = FakeHost::showing(screen).in_directory("/work/repo");
        world.files = FakeFiles::default().with_origin("git@github.com:o/r.git");
        world.run(&settings(), Some(&geometry()), None);
        assert_eq!(
            world.launcher.opened.borrow()[0].0,
            "https://github.com/o/r/commit/8b1a9953c4611299a820df69698463c3ca01599d"
        );

        let mut world = World::new(&[Key::Hint('a', Modifier::Main)]);
        world.host = FakeHost::showing(screen).in_directory("/work/repo");
        world.files = FakeFiles::default().with_origin("git@gitlab.com:o/r.git");
        world.run(&settings(), Some(&geometry()), None);
        assert!(world.launcher.opened.borrow().is_empty());
        assert_eq!(
            world.clipboard.copied,
            vec!["8b1a9953c4611299a820df69698463c3ca01599d"]
        );
        assert_eq!(world.host.notified.borrow().len(), 1);
    }

    #[test]
    fn previews_are_short_single_lines() {
        assert_eq!(preview("short"), "short");
        assert_eq!(preview("first\nsecond"), "first…");
        assert_eq!(preview(&"x".repeat(50)), format!("{}…", "x".repeat(40)));
    }
}
