# Maintainer: Diren Deniz Yildirim <direndenizyildirim2008@gmail.com>

# The binary is `hive`; the package is `hive-explorer` because `hive` is taken
# on the AUR by apache-hive.
pkgname=hive-explorer
pkgver=0.1.0
pkgrel=1
pkgdesc="Minimal pastel explorer — a file manager for Hyprland"
arch=('x86_64')
url="https://github.com/DirenDenizYildirim/Hive-Explorer"
license=('MIT')
depends=('gtk4' 'libadwaita')
makedepends=('rust' 'cargo')
optdepends=(
  'gvfs: removable drives and the Devices sidebar section'
  'udisks2: automounting removable media'
)
provides=('hive')
source=()
sha256sums=()

prepare() {
  cd "$startdir"
  cargo fetch --locked
}

build() {
  cd "$startdir"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  cargo build --release --locked
}

check() {
  cd "$startdir"
  export RUSTUP_TOOLCHAIN=stable
  cargo test --release --locked
}

package() {
  cd "$startdir"

  install -Dm755 target/release/hive "$pkgdir/usr/bin/hive"

  install -Dm644 data/dev.diren.Hive.desktop \
    "$pkgdir/usr/share/applications/dev.diren.Hive.desktop"

  install -Dm644 resources/icons/scalable/apps/dev.diren.Hive.svg \
    "$pkgdir/usr/share/icons/hicolor/scalable/apps/dev.diren.Hive.svg"

  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
