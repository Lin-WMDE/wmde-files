# Maintainer: WMDE <https://wmde.fun>
# Contributor: System76 <jeremy@system76.com> (original cosmic-files)
# Builds our fork Lin-WMDE/wmde-files (branch wmde; master mirrors pop-os upstream).
pkgname=wmde-files
pkgver=1.2.0
pkgrel=1
pkgdesc="WMDE Files - file manager for the WMDE desktop (fork of cosmic-files)"
arch=('x86_64')
url="https://wmde.fun"
license=('GPL-3.0-only')
# depends: readelf NEEDED -> glib2/libxkbcommon/gcc-libs/glibc; wayland, mesa,
# fontconfig and freetype2 are dlopen'd by libcosmic at runtime. Verify with namcap.
depends=('glibc' 'gcc-libs' 'glib2' 'libxkbcommon' 'wayland' 'mesa' 'fontconfig' 'freetype2')
optdepends=('gvfs: mount removable and network locations'
            'cosmic-icons: COSMIC icon theme')
# makedepends: same toolchain/libs that build the fork in Docker (Dockerfile.build) + glib2.
makedepends=('rust' 'cargo' 'just' 'git' 'clang' 'lld' 'pkgconf' 'glib2' 'mesa' 'wayland'
             'libxkbcommon' 'fontconfig' 'freetype2' 'expat' 'zstd')
provides=('cosmic-files')
conflicts=('cosmic-files')
replaces=('cosmic-files')
source=("$pkgname::git+https://github.com/Lin-WMDE/wmde-files.git#branch=wmde")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  git describe --long --tags --abbrev=7 2>/dev/null | sed 's/^epoch-//;s/^v//;s/\([^-]*-g\)/r\1/;s/-/./g' ||
    printf '1.2.0.r%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
}

build() {
  cd "$srcdir/$pkgname"
  # x86-64-v3 (AVX2/BMI2) baseline for the WMDE repo; runs on Haswell+ (and the VM).
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu=x86-64-v3"
  # link the system libzstd; the vendored zstd-sys build drops ZSTD_endStream under lld
  export ZSTD_SYS_USE_PKG_CONFIG=1
  cargo build --release --workspace   # root wmde-files + wmde-files-applet member
}

package() {
  cd "$srcdir/$pkgname"
  # installs wmde-files{,-applet} to /usr/bin and the fun.wmde.files
  # .desktop/metainfo/icons to /usr/share
  just rootdir="$pkgdir" prefix=/usr install
  # compat: `cosmic-files` name for by-name launches and to back provides=(cosmic-files)
  ln -s wmde-files "$pkgdir/usr/bin/cosmic-files"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
