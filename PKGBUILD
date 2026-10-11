# Maintainer: WMDE <https://wmde.fun>
# Contributor: Bryan Hyland and System76 (original cosmic-viewer)
#
# Builds our fork Lin-WMDE/wmde-viewer (branch wmde; master mirrors pop-os/cosmic-viewer).
#
# The viewer is what double-clicking a picture reaches. The association itself lives in
# wmde-session (wmde-mimeapps.list); only the types the viewer declares in MimeType= of
# res/fun.wmde.Viewer.desktop may be assigned to it there.
pkgname=wmde-viewer
pkgver=0.1.0
pkgrel=1
pkgdesc="WMDE Viewer - image viewer for the WMDE desktop (fork of cosmic-viewer)"
arch=('x86_64')
url="https://wmde.fun"
license=('GPL-3.0-only')
# runtime: wayland client (libcosmic winit/wayland). Vulkan is reached through dlopen,
# so makepkg cannot see it in ldd - the loader and a driver come from wmde-desktop.
#
# libheif and libjpeg-turbo are linked from the system (Cargo.toml turns off the
# embedded libheif and the cmake build of libjpeg-turbo): HEIC/HEIF go through libheif,
# JPEG through libturbojpeg. Check the final list against readelf -d after the first build.
#
# libxkbcommon is NEEDED as well, not dlopen'd: winit resolves the keyboard map through it.
depends=('glibc' 'gcc-libs' 'libxkbcommon' 'wayland' 'libheif' 'libjpeg-turbo')
optdepends=('wmde-icons-base: WMDE icon theme')
makedepends=('rust' 'cargo' 'just' 'git' 'wayland' 'clang' 'lld' 'pkgconf' 'libheif'
             'libjpeg-turbo')
source=("$pkgname::git+https://github.com/Lin-WMDE/wmde-viewer.git#branch=wmde")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  # WMDE version: <COSMIC major.minor>.<WMDE release>.<commits since the nearest tag>.g<short>.
  local desc
  desc=$(git describe --long --tags --abbrev=7 2>/dev/null || true)
  if [ -n "$desc" ]; then
    printf '1.10.1.%s.g%s' "$(printf '%s' "$desc" | sed -E 's/.*-([0-9]+)-g[0-9a-f]+$/\1/')" "$(git rev-parse --short=7 HEAD)"
  else
    printf '1.10.1.%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
  fi
}

build() {
  cd "$srcdir/$pkgname"
  # x86-64-v3 (AVX2/BMI2) baseline for the WMDE repo; runs on Haswell+ (and the VM).
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu=x86-64-v3"
  just build-release
}

check() {
  cd "$srcdir/$pkgname"
  # The dev profile on purpose: the release profile aborts on panic, and the test harness
  # needs to unwind to report a failure. No pipe after cargo: makepkg runs check() without
  # pipefail, and a pipe would swallow a failing test run.
  cargo test --locked
}

package() {
  cd "$srcdir/$pkgname"
  # installs /usr/bin/wmde-viewer, the fun.wmde.Viewer desktop entry and metainfo, and the
  # seven hicolor sizes of its icon. The icon name is the application id, which is also
  # what the window reports - a mismatch leaves the task bar button blank.
  just rootdir="$pkgdir" prefix=/usr install
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
