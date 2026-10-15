//! Opening things and running the user's commands, with real processes.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::domain::open::{PathKind, Surroundings};
use crate::usecases::ports::{FileSystem, Launcher, PortError};

/// `open` / `xdg-open` and shell command lines.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLauncher;

/// The system opener's argv for `target`. The target is always one argument
/// of its own and never read as an option: macOS `open` takes `--`, and
/// `xdg-open`, which does not, gets no target starting with `-`.
fn open_argv(target: &str) -> Result<Vec<String>, PortError> {
    if cfg!(target_os = "macos") {
        Ok(vec!["open".into(), "--".into(), target.into()])
    } else if target.starts_with('-') {
        Err(PortError::new(
            "refusing to open a target that starts with -",
        ))
    } else {
        Ok(vec!["xdg-open".into(), target.into()])
    }
}

/// Shows `path` in its folder: Finder selects it on macOS; elsewhere the
/// folder is opened.
fn reveal_argv(path: &Path) -> Result<Vec<String>, PortError> {
    if !path.is_absolute() {
        return Err(PortError::new("refusing to reveal a relative path"));
    }
    if cfg!(target_os = "macos") {
        Ok(vec![
            "open".into(),
            "-R".into(),
            "--".into(),
            path.to_string_lossy().into_owned(),
        ])
    } else {
        let folder = path.parent().unwrap_or(path);
        Ok(vec![
            "xdg-open".into(),
            folder.to_string_lossy().into_owned(),
        ])
    }
}

fn run_opener(argv: &[String], cwd: Option<&Path>) -> Result<(), PortError> {
    let (program, args) = argv.split_first().expect("an opener argv has a program");
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let status = command
        .status()
        .map_err(|error| PortError::new(format!("cannot run `{program}`: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(PortError::new(format!("`{program}` exited with an error")))
    }
}

impl Launcher for SystemLauncher {
    fn open(&self, target: &str, cwd: Option<&Path>) -> Result<(), PortError> {
        run_opener(&open_argv(target)?, cwd)
    }

    fn reveal(&self, path: &Path) -> Result<(), PortError> {
        run_opener(&reveal_argv(path)?, None)
    }

    fn run(
        &self,
        command_line: &str,
        stdin: &str,
        env: &[(&str, String)],
        cwd: Option<&Path>,
    ) -> Result<(), PortError> {
        let argv = shell_words::split(command_line)
            .map_err(|error| PortError::new(format!("cannot parse `{command_line}`: {error}")))?;
        let Some((program, args)) = argv.split_first() else {
            return Ok(());
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        let mut child = command
            .spawn()
            .map_err(|error| PortError::new(format!("cannot run `{command_line}`: {error}")))?;
        if let Some(mut pipe) = child.stdin.take() {
            // A command that exits early closes the pipe; its exit status is
            // the verdict, not the broken pipe.
            let _ = pipe.write_all(stdin.as_bytes());
        }
        let status = child
            .wait()
            .map_err(|error| PortError::new(format!("cannot run `{command_line}`: {error}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(PortError::new(format!(
                "`{command_line}` exited with an error"
            )))
        }
    }
}

/// Replaces this process with `argv`; returns only if that failed.
pub fn exec(argv: &[String], cwd: Option<&Path>) -> std::io::Error {
    let Some((program, args)) = argv.split_first() else {
        return std::io::Error::other("empty command");
    };
    let mut command = Command::new(program);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command.exec()
}

/// The real file system, and `git` for the remote of a repository.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemFiles;

impl Surroundings for SystemFiles {
    fn kind(&self, path: &Path) -> PathKind {
        match std::fs::metadata(path) {
            Err(_) => PathKind::Missing,
            Ok(meta) if meta.is_dir() => PathKind::Directory,
            Ok(meta) if meta.permissions().mode() & 0o111 != 0 => PathKind::Executable,
            Ok(_) => PathKind::File,
        }
    }

    fn real_path(&self, path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn git_origin(&self, dir: &Path) -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["remote", "get-url", "origin"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let url = String::from_utf8(output.stdout).ok()?;
        Some(url.trim().to_string()).filter(|url| !url.is_empty())
    }

    fn subdirectories(&self, dir: &Path) -> Vec<PathBuf> {
        // Enough for any folder of projects; a huge folder is cut short
        // rather than read whole on every pick.
        const MAX_ENTRIES: usize = 1000;
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut folders: Vec<PathBuf> = entries
            .take(MAX_ENTRIES)
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        folders
    }
}

impl FileSystem for SystemFiles {
    fn home_dir(&self) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("herdr-fingers-{name}-{unique}"))
    }

    #[test]
    fn a_command_gets_the_text_on_stdin_and_the_variables_in_its_environment() {
        let target = temp_file("launcher");
        let command_line = format!(
            "sh -c 'printf \"%s|%s|\" \"$MODIFIER\" \"$HINT\" > {0}; cat >> {0}'",
            target.display()
        );
        SystemLauncher
            .run(
                &command_line,
                "one two",
                &[("MODIFIER", "alt".into()), ("HINT", "ab".into())],
                None,
            )
            .unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "alt|ab|one two");
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn a_command_runs_in_the_given_directory() {
        let target = temp_file("cwd");
        let command_line = format!("sh -c 'pwd > {}'", target.display());
        let dir = std::env::temp_dir().canonicalize().unwrap();
        SystemLauncher
            .run(&command_line, "", &[], Some(&dir))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&target).unwrap().trim(),
            dir.to_str().unwrap()
        );
        let _ = std::fs::remove_file(target);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = temp_file(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn the_opener_gets_the_target_as_one_argument_never_as_an_option() {
        let tricky = "/tmp/a b'c\"$(touch x)";
        let argv = open_argv(tricky).unwrap();
        assert_eq!(argv.last().map(String::as_str), Some(tricky));
        if cfg!(target_os = "macos") {
            assert_eq!(argv, vec!["open", "--", tricky]);
            assert_eq!(open_argv("-rf").unwrap(), vec!["open", "--", "-rf"]);
        } else {
            assert_eq!(argv, vec!["xdg-open", tricky]);
            assert!(open_argv("-rf").is_err());
        }
    }

    #[test]
    fn revealing_shows_the_file_in_its_folder() {
        let argv = reveal_argv(Path::new("/w/run me")).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(argv, vec!["open", "-R", "--", "/w/run me"]);
        } else {
            assert_eq!(argv, vec!["xdg-open", "/w"]);
        }
        assert!(reveal_argv(Path::new("-rf")).is_err());
    }

    #[test]
    fn files_are_told_apart_by_kind() {
        let dir = temp_dir("kinds");
        let plain = dir.join("notes.md");
        let script = dir.join("run");
        std::fs::write(&plain, "x").unwrap();
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(SystemFiles.kind(&dir), PathKind::Directory);
        assert_eq!(SystemFiles.kind(&plain), PathKind::File);
        assert_eq!(SystemFiles.kind(&script), PathKind::Executable);
        assert_eq!(SystemFiles.kind(&dir.join("gone")), PathKind::Missing);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn subdirectories_are_the_folders_directly_inside_sorted() {
        let dir = temp_dir("subdirectories");
        std::fs::create_dir_all(dir.join("b/deep")).unwrap();
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("file"), "x").unwrap();
        assert_eq!(
            SystemFiles.subdirectories(&dir),
            vec![dir.join("a"), dir.join("b")]
        );
        assert!(SystemFiles.subdirectories(&dir.join("gone")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_link_leads_to_its_real_path() {
        let dir = temp_dir("links");
        let bundle = dir.join("Calc.app");
        std::fs::create_dir_all(&bundle).unwrap();
        std::os::unix::fs::symlink(&bundle, dir.join("docs")).unwrap();
        assert_eq!(SystemFiles.real_path(&dir.join("docs")), bundle);
        assert_eq!(SystemFiles.real_path(&dir.join("gone")), dir.join("gone"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_origin_remote_is_read_from_any_folder_of_the_repository() {
        let dir = temp_dir("repo");
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).success());
        assert_eq!(SystemFiles.git_origin(&dir), None);
        assert!(git(&["remote", "add", "origin", "git@github.com:o/r.git"]).success());
        std::fs::create_dir_all(dir.join("src")).unwrap();
        assert_eq!(
            SystemFiles.git_origin(&dir.join("src")),
            Some("git@github.com:o/r.git".to_string())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_open_error_does_not_repeat_the_target() {
        let target = temp_file("missing-secret");
        let error = SystemLauncher
            .open(target.to_str().unwrap(), None)
            .unwrap_err();
        assert!(!error.0.contains("missing-secret"));
    }

    #[test]
    fn a_failing_or_unparsable_command_is_reported() {
        let error = SystemLauncher
            .run("sh -c 'exit 2'", "x", &[], None)
            .unwrap_err();
        assert!(error.0.contains("exited with an error"));
        let error = SystemLauncher
            .run("sh -c 'unclosed", "", &[], None)
            .unwrap_err();
        assert!(error.0.contains("cannot parse"));
        assert!(SystemLauncher.run("", "", &[], None).is_ok());
    }
}
