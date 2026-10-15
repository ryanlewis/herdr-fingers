//! `herdr-plugin.toml` is what Herdr reads; it must agree with the crate.

use std::path::Path;

use herdr_fingers::adapters::herdr::DEFAULT_PLUGIN_ID;
use herdr_fingers::usecases::pick::EDITOR_ENTRYPOINT;
use herdr_fingers::usecases::start::OVERLAY_TITLE;

fn manifest() -> toml::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("herdr-plugin.toml");
    toml::from_str(&std::fs::read_to_string(path).expect("manifest exists")).expect("valid TOML")
}

#[test]
fn the_manifest_and_the_crate_share_one_version_and_one_id() {
    let manifest = manifest();
    assert_eq!(
        manifest["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(manifest["id"].as_str(), Some(DEFAULT_PLUGIN_ID));
}

#[test]
fn the_overlay_pane_carries_the_title_the_start_action_looks_for() {
    let manifest = manifest();
    let panes = manifest["panes"].as_array().expect("panes");
    let overlay = panes
        .iter()
        .find(|pane| pane["id"].as_str() == Some("overlay"))
        .expect("an overlay pane");
    assert_eq!(overlay["title"].as_str(), Some(OVERLAY_TITLE));
    assert_eq!(overlay["placement"].as_str(), Some("overlay"));
    assert_eq!(
        overlay["command"][1].as_str(),
        Some("ui"),
        "the overlay pane must run the `ui` subcommand"
    );
}

#[test]
fn the_start_action_runs_the_release_binary() {
    let manifest = manifest();
    let actions = manifest["actions"].as_array().expect("actions");
    let start = actions
        .iter()
        .find(|action| action["id"].as_str() == Some("start"))
        .expect("a start action");
    assert_eq!(
        start["command"][0].as_str(),
        Some("./target/release/herdr-fingers")
    );
    assert_eq!(start["command"][1].as_str(), Some("start"));
    assert_eq!(
        manifest["build"][0]["command"][0].as_str(),
        Some("./install.sh"),
        "install.sh puts the binary at ./target/release, downloaded or built"
    );
}

#[test]
fn the_editor_pane_is_a_split_running_the_edit_subcommand() {
    let manifest = manifest();
    let panes = manifest["panes"].as_array().expect("panes");
    let editor = panes
        .iter()
        .find(|pane| pane["id"].as_str() == Some(EDITOR_ENTRYPOINT))
        .expect("an editor pane");
    assert_eq!(editor["placement"].as_str(), Some("split"));
    let command = editor["command"].as_array().expect("a command");
    let script = command
        .last()
        .and_then(|word| word.as_str())
        .expect("a script");
    assert!(
        script.ends_with("/target/release/herdr-fingers\" edit"),
        "the editor pane must run the `edit` subcommand: {script}"
    );
}

#[test]
fn the_links_action_starts_the_overlay_in_link_mode() {
    let manifest = manifest();
    let actions = manifest["actions"].as_array().expect("actions");
    let links = actions
        .iter()
        .find(|action| action["id"].as_str() == Some("links"))
        .expect("a links action");
    let command: Vec<&str> = links["command"]
        .as_array()
        .expect("command")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    assert_eq!(
        command,
        vec!["./target/release/herdr-fingers", "start", "--links"]
    );
}
