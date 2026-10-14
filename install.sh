#!/bin/sh
# Puts the herdr-fingers binary at ./target/release/herdr-fingers, the path
# herdr-plugin.toml runs. It downloads the release binary for this platform
# and checks its SHA-256; when there is no binary for the platform, or the
# download or the check fails, it builds from source with cargo instead.
#
#   ./install.sh                         install
#   ./install.sh --print-asset [OS ARCH] print the release asset name for this
#                                        machine (or for `uname -s` OS and
#                                        `uname -m` ARCH) and exit
#
# HERDR_FINGERS_RELEASE_URL replaces the download directory, e.g. to test
# against a local copy of a release.
set -eu

cd "$(dirname "$0")"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' herdr-plugin.toml | head -n 1)
binary=target/release/herdr-fingers

log() {
	echo "herdr-fingers: $*" >&2
}

asset_suffix() {
	case "$1/$2" in
	Darwin/arm64) echo macos-aarch64 ;;
	Darwin/x86_64) echo macos-x86_64 ;;
	Linux/x86_64) echo linux-x86_64 ;;
	Linux/aarch64 | Linux/arm64) echo linux-aarch64 ;;
	*) return 1 ;;
	esac
}

asset_name() {
	suffix=$(asset_suffix "$1" "$2") || return 1
	echo "herdr-fingers-v$version-$suffix.tar.gz"
}

# A stalled network fails, and so falls back to cargo, instead of hanging the
# install; a slow download that keeps moving is left to finish. BusyBox wget
# knows only the short -T option.
download() {
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL --connect-timeout 15 --speed-limit 1024 --speed-time 30 -o "$2" "$1"
	elif command -v wget >/dev/null 2>&1; then
		wget -q -T 30 -O "$2" "$1"
	else
		log "neither curl nor wget is installed"
		return 1
	fi
}

verify() {
	if command -v shasum >/dev/null 2>&1; then
		(cd "$1" && shasum -a 256 -c "$2.sha256" >/dev/null)
	elif command -v sha256sum >/dev/null 2>&1; then
		(cd "$1" && sha256sum -c "$2.sha256" >/dev/null)
	else
		log "neither shasum nor sha256sum is installed"
		return 1
	fi
}

install_release() {
	asset=$(asset_name "$(uname -s)" "$(uname -m)") || {
		log "no release binary for $(uname -s) $(uname -m)"
		return 1
	}
	base=${HERDR_FINGERS_RELEASE_URL:-https://github.com/ryanlewis/herdr-fingers/releases/download/v$version}
	tmp=$(mktemp -d) || return 1
	trap 'rm -rf "$tmp"' EXIT
	trap 'exit 1' HUP INT TERM

	log "downloading $asset"
	download "$base/$asset" "$tmp/$asset" || return 1
	download "$base/$asset.sha256" "$tmp/$asset.sha256" || return 1
	verify "$tmp" "$asset" || {
		log "checksum of $asset does not match"
		return 1
	}
	tar -xzf "$tmp/$asset" -C "$tmp" || return 1
	unpacked="$tmp/${asset%.tar.gz}/herdr-fingers"
	[ -f "$unpacked" ] || {
		log "$asset has no herdr-fingers binary"
		return 1
	}
	# A binary built against a newer libc, or a 64-bit binary under a 32-bit
	# userland, matches uname but cannot start.
	"$unpacked" --version >/dev/null 2>&1 || {
		log "the release binary does not run on this machine"
		return 1
	}
	# `set -e` does not apply inside `install_release || ...`, hence the
	# explicit chain. Copy then rename, so an interrupted install never leaves
	# half a binary.
	mkdir -p target/release &&
		cp "$unpacked" "$binary.part" &&
		chmod 755 "$binary.part" &&
		mv -f "$binary.part" "$binary" || return 1
	log "installed $binary $version"
}

build_from_source() {
	command -v cargo >/dev/null 2>&1 || {
		log "cargo is not installed, so it cannot build from source either; install Rust from https://rustup.rs and try again"
		exit 1
	}
	log "building from source with cargo"
	cargo build --release --locked
}

if [ "${1:-}" = "--print-asset" ]; then
	asset_name "${2:-$(uname -s)}" "${3:-$(uname -m)}"
	exit
fi

install_release || build_from_source
