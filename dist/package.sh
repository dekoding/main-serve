#!/usr/bin/env bash
# Build Linux packages for main-serve.
# Usage: ./dist/package.sh [deb|rpm|arch|all]
#
# Prerequisites:
#   cargo install cargo-deb cargo-generate-rpm
#
# For Arch, this script only validates the PKGBUILD. Actual Arch package
# building requires makepkg in an Arch environment.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
echo "==> Building main-serve v${VERSION} packages"

build_release() {
    echo "==> Building release binary..."
    cargo build --release
    echo "    Binary: target/release/main-serve"
}

build_deb() {
    echo "==> Building .deb package..."
    if ! command -v cargo deb &>/dev/null; then
        echo "    ERROR: cargo-deb not installed. Run: cargo install cargo-deb"
        return 1
    fi
    cargo deb --no-build
    local deb
    deb=$(ls -1t target/debian/*.deb 2>/dev/null | head -1)
    echo "    .deb: $deb"
}

build_rpm() {
    echo "==> Building .rpm package..."
    if ! command -v cargo generate-rpm &>/dev/null; then
        echo "    ERROR: cargo-generate-rpm not installed. Run: cargo install cargo-generate-rpm"
        return 1
    fi
    cargo generate-rpm
    local rpm
    rpm=$(ls -1t target/generate-rpm/*.rpm 2>/dev/null | head -1)
    echo "    .rpm: $rpm"
}

check_arch() {
    echo "==> Validating Arch PKGBUILD..."
    if [ ! -f "dist/arch/PKGBUILD" ]; then
        echo "    ERROR: dist/arch/PKGBUILD not found"
        return 1
    fi
    if [ ! -f "dist/main-serve.install" ]; then
        echo "    ERROR: dist/main-serve.install not found"
        return 1
    fi
    if [ ! -f "dist/main-serve.service" ]; then
        echo "    ERROR: dist/main-serve.service not found"
        return 1
    fi
    if [ ! -f "dist/main-serve.sysusers" ]; then
        echo "    ERROR: dist/main-serve.sysusers not found"
        return 1
    fi
    if [ ! -f "dist/main-serve.tmpfiles" ]; then
        echo "    ERROR: dist/main-serve.tmpfiles not found"
        return 1
    fi
    # Basic syntax check
    bash -n "dist/arch/PKGBUILD" 2>&1
    echo "    PKGBUILD syntax: OK"
}

TARGET="${1:-all}"

case "$TARGET" in
    deb)
        build_release
        build_deb
        ;;
    rpm)
        build_release
        build_rpm
        ;;
    arch)
        check_arch
        ;;
    all)
        build_release
        build_deb
        build_rpm
        check_arch
        ;;
    *)
        echo "Usage: $0 [deb|rpm|arch|all]"
        exit 1
        ;;
esac

echo "==> Done."
