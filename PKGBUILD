# Maintainer: WMDE <https://wmde.fun>
# Contributor: System76 <jeremy@system76.com> (original cosmic-files)
# Builds our fork Lin-WMDE/wmde-files (branch wmde; master mirrors pop-os upstream).
pkgname=wmde-files
pkgver=1.2.0
pkgrel=4
pkgdesc="WMDE Files - file manager for the WMDE desktop (fork of cosmic-files)"
arch=('x86_64')
url="https://wmde.fun"
license=('GPL-3.0-only')
# depends: readelf NEEDED -> glib2/libxkbcommon/gcc-libs/glibc; wayland, mesa,
# fontconfig and freetype2 are dlopen'd by libcosmic at runtime. Verify with namcap.
# gvfs + gvfs-smb are hard deps: the file manager mounts removable and network
# locations (SMB/FTP/NFS/SFTP/DAV) through gvfs, and the SMB backend lives in the
# separate gvfs-smb package (pulls smbclient). Required so SMB works out of the box.
# avahi provides avahi-browse used to populate network:/// via mDNS (needs avahi-daemon
# running - enabled at the WMDE session/stack level).
depends=('glibc' 'gcc-libs' 'glib2' 'libxkbcommon' 'wayland' 'mesa' 'fontconfig' 'freetype2'
         'gvfs' 'gvfs-smb' 'avahi')
optdepends=('cosmic-icons: COSMIC icon theme')
# makedepends: same toolchain/libs that build the fork in Docker (Dockerfile.build) + glib2.
makedepends=('rust' 'cargo' 'just' 'git' 'clang' 'lld' 'pkgconf' 'glib2' 'mesa' 'wayland'
             'libxkbcommon' 'fontconfig' 'freetype2' 'expat' 'zstd')
source=("$pkgname::git+https://github.com/Lin-WMDE/wmde-files.git#branch=wmde")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  # WMDE unified version: 1.4 (libcosmic base) . <commits since nearest tag> . g<short>.
  local desc
  desc=$(git describe --long --tags --abbrev=7 2>/dev/null || true)
  if [ -n "$desc" ]; then
    printf '1.4.%s.g%s' "$(printf '%s' "$desc" | sed -E 's/.*-([0-9]+)-g[0-9a-f]+$/\1/')" "$(git rev-parse --short=7 HEAD)"
  else
    printf '1.4.%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
  fi
}

build() {
  cd "$srcdir/$pkgname"
  # x86-64-v3 (AVX2/BMI2) baseline for the WMDE repo; runs on Haswell+ (and the VM).
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu=x86-64-v3"
  # link the system libzstd; the vendored zstd-sys build drops ZSTD_endStream under lld
  export ZSTD_SYS_USE_PKG_CONFIG=1
  # Separate invocations so feature unification does not leak the applet's
  # `desktop-applet` feature into the app binary (which would compile out the
  # recents watcher). Two builds share the target dir; correctness over speed.
  cargo build --release -p wmde-files
  cargo build --release -p wmde-files-applet
}

package() {
  cd "$srcdir/$pkgname"
  # installs wmde-files{,-applet} to /usr/bin and the fun.wmde.files
  # .desktop/metainfo/icons to /usr/share
  just rootdir="$pkgdir" prefix=/usr install
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
