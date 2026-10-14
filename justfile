name := 'wmde-files'
export APPID := 'fun.wmde.files'
# Чтобы shell() работал и на Windows с Git Bash; на Linux игнорируется
set windows-shell := ["C:\\Program Files\\Git\\bin\\sh.exe", "-cu"]

rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))

export INSTALL_DIR := base-dir / 'share'

cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bin-src := cargo-target-dir / 'release' / name
bin-dst := base-dir / 'bin' / name

thumbnailer-bin-name := name + '-thumbnailer'
thumbnailer-bin-src := cargo-target-dir / 'release' / thumbnailer-bin-name
thumbnailer-bin-dst := base-dir / 'bin' / thumbnailer-bin-name

applet-name := name + '-applet'
applet-src := cargo-target-dir / 'release' / applet-name
applet-dst := base-dir / 'bin' / applet-name

desktop := APPID + '.desktop'
desktop-src := 'target/xdgen' / desktop
desktop-dst := clean(rootdir / prefix) / 'share' / 'applications' / desktop

metainfo := APPID + '.metainfo.xml'
metainfo-src := 'target/xdgen' / metainfo
metainfo-dst := clean(rootdir / prefix) / 'share' / 'metainfo' / metainfo
# The key bindings the file manager handles itself, read by the settings app.
shortcuts := APPID + '.ron'
shortcuts-src := 'res' / 'app-shortcuts' / shortcuts
shortcuts-dst := clean(rootdir / prefix) / 'share' / 'wmde' / 'app-shortcuts' / shortcuts

# WMDE: the secondary windows run under their own app ids, and the panel needs an entry under
# each to find an icon. Static, unlike the main entry, which xdgen generates.
window-desktops := APPID + '.dialog.desktop ' + APPID + '.operations.desktop'
applications-dst := clean(rootdir / prefix) / 'share' / 'applications'

thumbnailer := APPID + '.thumbnailer'
thumbnailer-src := 'res' / thumbnailer
thumbnailer-dst := clean(rootdir / prefix) / 'share' / 'thumbnailers' / thumbnailer

icons-src := 'res' / 'icons' / 'hicolor'
icons-dst := clean(rootdir / prefix) / 'share' / 'icons' / 'hicolor'
# WMDE: icon of the file operations window, alongside the icon of the manager itself.
operations-icon := APPID + '.operations'

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# Removes vendored dependencies
clean-vendor:
    rm -rf .cargo vendor vendor.tar

# `cargo clean` and removes vendored dependencies
clean-dist: clean clean-vendor

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}
    cargo build --package {{applet-name}} {{args}}
    cargo build --package {{thumbnailer-bin-name}} {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles release profile with vendored dependencies
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Runs a clippy check
check *args:
    cargo clippy --all-features {{args}} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Developer target
dev *args:
    cargo fmt
    just run {{args}}

# Run with debug logs
run *args:
    cargo build --release
    env RUST_LOG=wmde_files=debug RUST_BACKTRACE=full {{bin-src}} {{args}}

# Run tests
test *args:
    cargo test {{args}}

flamegraph *args:
    cargo flamegraph --release --bin wmde-files -- --no-daemon {{args}}
    xdg-open flamegraph.svg

heaptrack *args:
    #!/usr/bin/env bash
    set -ex
    rm -fv heaptrack.wmde-files.*
    cargo heaptrack --profile release-with-debug --bin wmde-files -- --no-daemon {{args}}
    zstd -dc < heaptrack.wmde-files.*.raw.zst | /usr/lib/heaptrack/libexec/heaptrack_interpret | zstd -c > heaptrack.wmde-files.zst
    heaptrack_gui heaptrack.wmde-files.zst

# Installs files
install:
    install -Dm0755 {{bin-src}} {{bin-dst}}
    install -Dm0755 {{thumbnailer-bin-src}} {{thumbnailer-bin-dst}}
    install -Dm0755 {{applet-src}} {{applet-dst}}
    install -Dm0644 {{desktop-src}} {{desktop-dst}}
    install -Dm0644 {{metainfo-src}} {{metainfo-dst}}
    install -Dm0644 {{thumbnailer-src}} {{thumbnailer-dst}}
    install -Dm0644 {{shortcuts-src}} {{shortcuts-dst}}
    for entry in {{window-desktops}}; do \
        install -Dm0644 "res/$entry" "{{applications-dst}}/$entry"; \
    done
    for size in `ls {{icons-src}}`; do \
        install -Dm0644 "{{icons-src}}/$size/apps/{{APPID}}.svg" "{{icons-dst}}/$size/apps/{{APPID}}.svg"; \
        install -Dm0644 "{{icons-src}}/$size/apps/{{operations-icon}}.svg" "{{icons-dst}}/$size/apps/{{operations-icon}}.svg"; \
    done

# Installs applet files
install-applet:
    install -Dm0755 {{applet-src}} {{applet-dst}}

# Uninstalls installed files
uninstall:
    rm -f {{bin-dst}} {{thumbnailer-bin-dst}} {{applet-dst}} {{thumbnailer-dst}}

# Vendor dependencies locally
vendor:
    #!/usr/bin/env bash
    mkdir -p .cargo
    cargo vendor --locked --sync Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    echo >> .cargo/config.toml
    echo '[env]' >> .cargo/config.toml
    if [ -n "${SOURCE_DATE_EPOCH}" ]
    then
        source_date="$(date -d "@${SOURCE_DATE_EPOCH}" "+%Y-%m-%d")"
        echo "VERGEN_GIT_COMMIT_DATE = \"${source_date}\"" >> .cargo/config.toml
    fi
    if [ -n "${SOURCE_GIT_HASH}" ]
    then
        echo "VERGEN_GIT_SHA = \"${SOURCE_GIT_HASH}\"" >> .cargo/config.toml
    fi
    tar pcf vendor.tar .cargo vendor
    rm -rf .cargo vendor

# Extracts vendored dependencies
vendor-extract:
    rm -rf vendor
    tar pxf vendor.tar

# Версия пакета как pkgver() в PKGBUILD: 1.10.0.<commits>.g<sha>.
# Теги epoch-* дают только счётчик через git describe, fallback — rev-list.
wmde-version := shell('desc=$(git describe --long --tags --abbrev=7 2>/dev/null || true); if [ -n "$desc" ]; then printf "1.10.0.%s.g%s" "$(printf "%s" "$desc" | sed -E "s/.*-([0-9]+)-g[0-9a-f]+$/\\1/")" "$(git rev-parse --short=7 HEAD)"; else printf "1.10.0.%s.g%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"; fi')
# CPU-baseline: пусто = чистый x86-64 (для deb/rpm по умолчанию), для v3: WMDE_CPU=x86-64-v3
wmde-cpu := env('WMDE_CPU', '')

# Раскладывает собранное в target/staging (тот же набор, что PKGBUILD кладёт в $pkgdir).
# Три раздельных вызова cargo — как build() в PKGBUILD: иначе feature unification
# протекает фичей applet'а desktop-applet в бинарь приложения (глушит recents watcher).
# ZSTD_SYS_USE_PKG_CONFIG=1 — системный libzstd, иначе vendored zstd-sys роняет символы под lld.
package-staging staging="target/staging":
    #!/usr/bin/env bash
    set -e
    rm -rf {{staging}}
    if [ -n "{{wmde-cpu}}" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu={{wmde-cpu}}"; fi
    export ZSTD_SYS_USE_PKG_CONFIG=1
    cargo build --release -p wmde-files
    cargo build --release -p wmde-files-applet
    cargo build --release -p wmde-files-thumbnailer
    just rootdir="{{staging}}" prefix=/usr install
    # Categories=WMDE в .desktop — незарегистрированная категория (E rpmlint).
    # Правим только стейджинг в X-WMDE по спеке, сырцы не трогаем.
    sed -i 's/^Categories=WMDE;/Categories=X-WMDE;/' "{{staging}}/usr/share/applications/"*.desktop
    # makepkg на Arch стрипает бинарники сам, в deb/rpm это делаем руками
    strip --strip-unneeded "{{staging}}/usr/bin/"* 2>/dev/null || true
    just package-doc "{{staging}}"

# doc для deb/rpm: копирайт + changelog.yaml (nFPM сам разложит changelog
# в changelog.Debian.gz для deb и в %changelog для rpm)
package-doc staging="target/staging":
    #!/usr/bin/env bash
    set -e
    mkdir -p "{{staging}}/usr/share/doc/{{name}}"
    cp packaging/copyright "{{staging}}/usr/share/doc/{{name}}/copyright"
    chmod 644 "{{staging}}/usr/share/doc/{{name}}/copyright"
    printf -- '- semver: %s\n  date: %s\n  packager: WMDE <wmde@wmde.fun>\n  urgency: low\n  distribution: unstable\n  deb:\n    urgency: low\n    distributions:\n      - unstable\n  changes:\n    - note: Automated build via nFPM (see INSTALL.deb-rpm.md).\n' "{{wmde-version}}-1" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > target/changelog.yaml

# Стейджинг из вендора: сборка без сети к git-зависимостям.
# vendor.tar генерим один раз при наличии сети (`just vendor`), сам тарболл
# в git не коммитится (см. .gitignore), на офлайн-машину его привозим файлом.
# Соседние чек-ауты нужны и тут: path-зависимости вендором не покрываются.
package-staging-vendored staging="target/staging":
    #!/usr/bin/env bash
    set -e
    rm -rf {{staging}}
    if [ ! -f vendor.tar ]; then just vendor; fi
    if [ -n "{{wmde-cpu}}" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu={{wmde-cpu}}"; fi
    export ZSTD_SYS_USE_PKG_CONFIG=1
    just vendor-extract
    cargo build --frozen --offline --release -p wmde-files
    cargo build --frozen --offline --release -p wmde-files-applet
    cargo build --frozen --offline --release -p wmde-files-thumbnailer
    just rootdir="{{staging}}" prefix=/usr install
    sed -i 's/^Categories=WMDE;/Categories=X-WMDE;/' "{{staging}}/usr/share/applications/"*.desktop
    strip --strip-unneeded "{{staging}}/usr/bin/"* 2>/dev/null || true
    just package-doc "{{staging}}"
    rm -rf vendor .cargo

# Собирает .deb через nFPM (нужен nfpm в PATH)
deb:
    #!/usr/bin/env bash
    set -e
    just package-staging
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager deb --target target/

# Собирает .rpm через nFPM (нужен nfpm в PATH)
rpm:
    #!/usr/bin/env bash
    set -e
    just package-staging
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager rpm --target target/

# .deb из вендора (офлайн-сборка)
deb-vendored:
    #!/usr/bin/env bash
    set -e
    just package-staging-vendored
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager deb --target target/

# .rpm из вендора (офлайн-сборка)
rpm-vendored:
    #!/usr/bin/env bash
    set -e
    just package-staging-vendored
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager rpm --target target/
