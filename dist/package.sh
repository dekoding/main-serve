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

# Extract pkgname and pkgver from PKGBUILD, not Cargo.toml
# PKGBUILD is what makepkg reads, so the versions must match
PKGBUILD="dist/arch/PKGBUILD"
PKGNAME=$(grep '^pkgname=' "$PKGBUILD" | head -1 | sed 's/^pkgname=//')
PKGVER=$(grep '^pkgver=' "$PKGBUILD" | head -1 | sed 's/^pkgver=//')
VERSION="$PKGVER"
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
    echo "==> Building Arch package..."
    if ! command -v makepkg &>/dev/null; then
        echo "    ERROR: makepkg not found. This script must be run in an Arch environment."
        return 1
    fi
    if [ ! -f "dist/arch/PKGBUILD" ]; then
        echo "    ERROR: dist/arch/PKGBUILD not found"
        return 1
    fi
    if [ ! -f "dist/arch/main-serve.install" ]; then
        echo "    ERROR: dist/arch/main-serve.install not found"
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

    build_release

    ORIGINAL_PKGBUILD="$(mktemp)"
    TARBALL="$(mktemp --suffix=.tar.gz)"
    PKGBUILD_DIR="$(cd dist/arch && pwd)"
    LOCAL_TARBALL="${PKGBUILD_DIR}/${PKGNAME}-${PKGVER}.tar.gz"

    cleanup() {
        rm -f "${PKGBUILD_DIR}/${PKGNAME}-${PKGVER}.tar.gz" 2>/dev/null || true
        rm -rf "${PKGBUILD_DIR}/src" "${PKGBUILD_DIR}/pkg" 2>/dev/null || true
        if [ -f "$ORIGINAL_PKGBUILD" ]; then
            cp "$ORIGINAL_PKGBUILD" "${PKGBUILD_DIR}/PKGBUILD" 2>/dev/null || true
            rm -f "$ORIGINAL_PKGBUILD" 2>/dev/null || true
        fi
        rm -f "$TARBALL" 2>/dev/null || true
    }
    trap cleanup EXIT

    cp "dist/arch/PKGBUILD" "$ORIGINAL_PKGBUILD"

    echo "    Creating source tarball ($PKGNAME-$PKGVER)..."
    git archive --prefix="${PKGNAME}-${PKGVER}/" -o "$TARBALL" HEAD

    local hash
    hash="$(sha256sum "$TARBALL" | awk '{print $1}')"

    cp "$TARBALL" "${PKGBUILD_DIR}/${PKGNAME}-${PKGVER}.tar.gz"

    echo "    Updating PKGBUILD for local tarball..."
    sed -e "s|^source=.*|source=(\"${PKGNAME}-${PKGVER}.tar.gz\")|" \
        -e "s|^sha256sums=.*|sha256sums=('$hash')|" \
        "$ORIGINAL_PKGBUILD" > "${PKGBUILD_DIR}/PKGBUILD"

    echo "    Running makepkg..."
    pushd "$PKGBUILD_DIR" > /dev/null
    if ! makepkg --noconfirm 2>&1; then
        echo "    ERROR: makepkg failed"
        popd > /dev/null
        exit 1
    fi
    local pkgfile
    pkgfile=$(ls -1t *.pkg.tar.* 2>/dev/null | head -1)
    echo "    .pkg.tar.zst: $pkgfile"
    popd > /dev/null

    echo "    Restoring original PKGBUILD and cleaning up..."
    cleanup
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
