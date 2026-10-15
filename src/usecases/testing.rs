//! In-memory stand-ins for the ports, shared by the use-case tests.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use super::ports::{
    Clipboard, FileSystem, Launcher, PaneHost, PickView, Picker, PortError, SplitPane,
};
use crate::domain::geometry::{Layout, OverlayGeometry, PanePlacement, Rect};
use crate::domain::open::{PathKind, Surroundings};
use crate::domain::session::{Key, Outcome, Session};
use crate::domain::settings::{Actions, SplitDirection};

/// One `open_overlay` call: plugin id, entrypoint, environment.
pub(crate) type OpenedOverlay = (String, String, BTreeMap<String, String>);

/// One `open_split` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenedSplit {
    pub plugin_id: String,
    pub entrypoint: String,
    pub beside: String,
    pub direction: SplitDirection,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

/// A host showing a fixed screen and recording everything asked of it.
pub(crate) struct FakeHost {
    pub screen: String,
    pub layout: Option<Layout>,
    pub label: Option<String>,
    pub cwd: Option<PathBuf>,
    pub reads_fail: bool,
    /// Link addresses by `(row, col)` of any cell they cover.
    pub links: BTreeMap<(usize, u16), String>,
    /// Set when the host cannot look links up, with its error.
    pub links_fail: Option<String>,
    pub link_calls: RefCell<Vec<(usize, u16)>>,
    pub layout_calls: RefCell<Vec<String>>,
    pub sent: RefCell<Vec<(String, String)>>,
    pub notified: RefCell<Vec<(String, String)>>,
    pub opened: RefCell<Vec<OpenedOverlay>>,
    pub splits: RefCell<Vec<OpenedSplit>>,
}

impl FakeHost {
    pub fn showing(screen: &str) -> Self {
        FakeHost {
            screen: screen.to_string(),
            layout: None,
            label: None,
            cwd: None,
            reads_fail: false,
            links: BTreeMap::new(),
            links_fail: None,
            link_calls: RefCell::new(Vec::new()),
            layout_calls: RefCell::new(Vec::new()),
            sent: RefCell::new(Vec::new()),
            notified: RefCell::new(Vec::new()),
            opened: RefCell::new(Vec::new()),
            splits: RefCell::new(Vec::new()),
        }
    }

    /// A 100×40 tab split in two, `pane_id` being the right half.
    pub fn split_layout(pane_id: &str) -> Layout {
        Layout {
            area: Rect::new(0, 0, 100, 40),
            zoomed: false,
            panes: vec![
                PanePlacement {
                    pane_id: "w1:p1".into(),
                    rect: Rect::new(0, 0, 50, 40),
                },
                PanePlacement {
                    pane_id: pane_id.into(),
                    rect: Rect::new(51, 0, 49, 40),
                },
            ],
        }
    }

    pub fn with_split_layout(pane_id: &str) -> Self {
        FakeHost::showing("").with_layout(FakeHost::split_layout(pane_id))
    }

    pub fn with_layout(mut self, layout: Layout) -> Self {
        self.layout = Some(layout);
        self
    }

    pub fn labelled(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    pub fn in_directory(mut self, cwd: &str) -> Self {
        self.cwd = Some(PathBuf::from(cwd));
        self
    }

    pub fn failing_reads(mut self) -> Self {
        self.reads_fail = true;
        self
    }

    /// A link to `url` drawn over `width` cells from `(row, col)`.
    pub fn with_link(mut self, row: usize, col: u16, width: u16, url: &str) -> Self {
        for offset in 0..width {
            self.links.insert((row, col + offset), url.to_string());
        }
        self
    }

    pub fn failing_links(mut self, error: &str) -> Self {
        self.links_fail = Some(error.to_string());
        self
    }
}

impl PaneHost for FakeHost {
    fn layout(&self, pane_id: &str) -> Result<Layout, PortError> {
        self.layout_calls.borrow_mut().push(pane_id.to_string());
        self.layout
            .clone()
            .ok_or_else(|| PortError::new("no layout"))
    }

    fn read_visible(&self, _pane_id: &str) -> Result<String, PortError> {
        if self.reads_fail {
            return Err(PortError::new("pane not found"));
        }
        Ok(self.screen.clone())
    }

    fn link_at(&self, _pane_id: &str, row: usize, col: u16) -> Result<Option<String>, PortError> {
        self.link_calls.borrow_mut().push((row, col));
        if let Some(error) = &self.links_fail {
            return Err(PortError::new(error));
        }
        Ok(self.links.get(&(row, col)).cloned())
    }

    fn pane_label(&self, _pane_id: &str) -> Result<Option<String>, PortError> {
        Ok(self.label.clone())
    }

    fn pane_cwd(&self, _pane_id: &str) -> Result<Option<PathBuf>, PortError> {
        Ok(self.cwd.clone())
    }

    fn open_overlay(
        &self,
        plugin_id: &str,
        entrypoint: &str,
        env: BTreeMap<String, String>,
    ) -> Result<String, PortError> {
        self.opened
            .borrow_mut()
            .push((plugin_id.to_string(), entrypoint.to_string(), env));
        Ok("w1:p9".to_string())
    }

    fn open_split(&self, split: &SplitPane<'_>) -> Result<String, PortError> {
        self.splits.borrow_mut().push(OpenedSplit {
            plugin_id: split.plugin_id.to_string(),
            entrypoint: split.entrypoint.to_string(),
            beside: split.beside.to_string(),
            direction: split.direction,
            cwd: split.cwd.map(Path::to_path_buf),
            env: split.env.clone(),
        });
        Ok("w1:p8".to_string())
    }

    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), PortError> {
        self.sent
            .borrow_mut()
            .push((pane_id.to_string(), text.to_string()));
        Ok(())
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), PortError> {
        self.notified
            .borrow_mut()
            .push((title.to_string(), body.to_string()));
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingClipboard {
    pub copied: Vec<String>,
}

impl Clipboard for RecordingClipboard {
    fn copy(&mut self, text: &str) -> Result<(), PortError> {
        self.copied.push(text.to_string());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ran {
    pub command_line: String,
    pub stdin: String,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct RecordingLauncher {
    pub opened: RefCell<Vec<(String, Option<PathBuf>)>>,
    pub revealed: RefCell<Vec<PathBuf>>,
    pub ran: RefCell<Vec<Ran>>,
}

impl Launcher for RecordingLauncher {
    fn open(&self, target: &str, cwd: Option<&Path>) -> Result<(), PortError> {
        self.opened
            .borrow_mut()
            .push((target.to_string(), cwd.map(Path::to_path_buf)));
        Ok(())
    }

    fn reveal(&self, path: &Path) -> Result<(), PortError> {
        self.revealed.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn run(
        &self,
        command_line: &str,
        stdin: &str,
        env: &[(&str, String)],
        cwd: Option<&Path>,
    ) -> Result<(), PortError> {
        self.ran.borrow_mut().push(Ran {
            command_line: command_line.to_string(),
            stdin: stdin.to_string(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            cwd: cwd.map(Path::to_path_buf),
        });
        Ok(())
    }
}

/// A file system holding only the paths it was given.
#[derive(Default)]
pub(crate) struct FakeFiles {
    pub kinds: BTreeMap<PathBuf, PathKind>,
    pub origin: Option<String>,
    pub home: Option<PathBuf>,
}

impl FakeFiles {
    pub fn with(mut self, path: &str, kind: PathKind) -> Self {
        self.kinds.insert(PathBuf::from(path), kind);
        self
    }

    pub fn with_origin(mut self, origin: &str) -> Self {
        self.origin = Some(origin.to_string());
        self
    }
}

impl Surroundings for FakeFiles {
    fn kind(&self, path: &Path) -> PathKind {
        self.kinds.get(path).copied().unwrap_or(PathKind::Missing)
    }

    fn real_path(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    fn git_origin(&self, _dir: &Path) -> Option<String> {
        self.origin.clone()
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

impl FileSystem for FakeFiles {
    fn home_dir(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

/// Feeds a fixed key sequence to the real session, picking up where the
/// previous call stopped; cancels when it runs out.
pub(crate) struct ScriptedPicker {
    keys: VecDeque<Key>,
    pub seen_notice: Option<String>,
    pub seen_geometry: Option<OverlayGeometry>,
    pub seen_legend: Option<(Actions, bool)>,
}

impl ScriptedPicker {
    pub fn new(keys: &[Key]) -> Self {
        ScriptedPicker {
            keys: keys.iter().copied().collect(),
            seen_notice: None,
            seen_geometry: None,
            seen_legend: None,
        }
    }
}

impl Picker for ScriptedPicker {
    fn pick(&mut self, view: &PickView<'_>, session: &mut Session) -> Result<Outcome, PortError> {
        self.seen_notice = view.notice.map(str::to_string);
        self.seen_geometry = view.geometry.cloned();
        self.seen_legend = Some((view.actions.clone(), view.show_legend));
        while let Some(key) = self.keys.pop_front() {
            match session.press(key) {
                Outcome::Continue => {}
                other => return Ok(other),
            }
        }
        Ok(Outcome::Cancelled)
    }
}
