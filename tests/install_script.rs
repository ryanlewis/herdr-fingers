//! `install.sh` is what `herdr plugin install` runs; it must ask the release
//! for the archive the release workflow names for each platform.

use std::path::Path;
use std::process::Command;

fn release_asset_for(os: &str, arch: &str) -> Option<String> {
    let output = Command::new("sh")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
        .args(["--print-asset", os, arch])
        .output()
        .expect("sh runs");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8");
    output.status.success().then(|| stdout.trim().to_owned())
}

#[test]
fn each_supported_platform_maps_to_its_release_archive() {
    let platforms = [
        ("Darwin", "arm64", "macos-aarch64"),
        ("Darwin", "x86_64", "macos-x86_64"),
        ("Linux", "x86_64", "linux-x86_64"),
        ("Linux", "aarch64", "linux-aarch64"),
        ("Linux", "arm64", "linux-aarch64"),
    ];
    for (os, arch, suffix) in platforms {
        let archive = format!(
            "herdr-fingers-v{}-{suffix}.tar.gz",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(release_asset_for(os, arch), Some(archive), "{os} {arch}");
    }
}

#[test]
fn an_unsupported_platform_has_no_release_archive() {
    assert_eq!(release_asset_for("FreeBSD", "amd64"), None);
    assert_eq!(release_asset_for("Linux", "riscv64"), None);
}
