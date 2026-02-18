#!/usr/bin/env bash
# ArmadAI installer — downloads the latest release binary for your platform.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/develop/install.sh | bash
#
# Options (via environment variables):
#   INSTALL_DIR   — where to install (default: ~/.local/bin)
#   VERSION       — specific version to install (default: latest)

set -euo pipefail

REPO="Dr0drigues/armadai"
BINARY_NAME="armadai"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

# Detect OS and architecture
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        *)      echo "Error: unsupported OS: $os" >&2; exit 1 ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *)              echo "Error: unsupported architecture: $arch" >&2; exit 1 ;;
    esac

    echo "${os}-${arch}"
}

# Get the latest release tag from GitHub
get_latest_version() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//'
    elif command -v wget &>/dev/null; then
        wget -qO- "$url" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//'
    else
        echo "Error: curl or wget is required" >&2
        exit 1
    fi
}

main() {
    local platform version artifact_name download_url tmp_dir

    platform="$(detect_platform)"
    artifact_name="armadai-${platform}"

    echo "ArmadAI Installer"
    echo "================="
    echo ""

    # Determine version
    if [ -n "${VERSION:-}" ]; then
        version="$VERSION"
        echo "Installing version: $version"
    else
        echo "Fetching latest version..."
        version="$(get_latest_version)"
        if [ -z "$version" ]; then
            echo "Error: could not determine latest version." >&2
            echo "You can specify a version with: VERSION=v0.1.0 $0" >&2
            exit 1
        fi
        echo "Latest version: $version"
    fi

    download_url="https://github.com/${REPO}/releases/download/${version}/${artifact_name}"
    echo "Platform: $platform"
    echo "Download: $download_url"
    echo ""

    # Download
    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT

    echo "Downloading..."
    if command -v curl &>/dev/null; then
        curl -fsSL -o "${tmp_dir}/${BINARY_NAME}" "$download_url"
    else
        wget -qO "${tmp_dir}/${BINARY_NAME}" "$download_url"
    fi

    # Install
    mkdir -p "$INSTALL_DIR"
    chmod +x "${tmp_dir}/${BINARY_NAME}"
    mv "${tmp_dir}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"

    echo "Installed to: ${INSTALL_DIR}/${BINARY_NAME}"

    # Check PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo ""
        echo "Warning: $INSTALL_DIR is not in your PATH."
        echo "Add it with:"
        echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.bashrc"
        echo "  # or for zsh:"
        echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.zshrc"
    fi

    echo ""
    echo "Done! Run 'armadai --version' to verify."
}

main "$@"
