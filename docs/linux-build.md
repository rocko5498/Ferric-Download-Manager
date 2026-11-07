# Linux Build & Packaging Notes (FQT-111)

## Dependencies (Ubuntu/Debian)

```bash
# Qt 6 development
sudo apt install qt6-base-dev qt6-declarative-dev qt6-shader-baker \
    libqt6svg6-dev qt6-tools-dev qt6-tools-dev-tools

# Build tools
sudo apt install build-essential cmake pkg-config

# Rust (via rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Additional
sudo apt install libssl-dev libclang-dev clang
```

## Dependencies (Fedora/RHEL)

```bash
sudo dnf install qt6-qtbase-devel qt6-qtdeclarative-devel \
    qt6-qtshadertools-devel qt6-qttools-devel
sudo dnf install openssl-devel clang-devel
```

## Build

```bash
cargo build -p ferric-qt -p ferricd -p ferric-host
```

## Package (AppImage)

```bash
# Install linuxdeployqt
# https://github.com/probonopd/linuxdeployqt

linuxdeployqt target/debug/ferric-qt -appimage \
    -qmldir crates/ferric-qt/qml
```

## Package (deb)

```bash
mkdir -p debian/usr/bin debian/usr/share/applications debian/usr/share/icons

cp target/debug/ferric-qt debian/usr/bin/
cp target/debug/ferricd debian/usr/bin/
cp target/debug/ferric-host debian/usr/bin/

# Desktop entry
cat > debian/usr/share/applications/ferric.desktop << 'EOF'
[Desktop Entry]
Type=Application
Name=Ferric
Exec=ferric-qt
Icon=ferric
Categories=Network;DownloadManager;
EOF
```

## Known limitations
- cxx-qt requires Qt 6.5+ with MSVC on Windows; on Linux, GCC or Clang works
- System tray requires `libappindicator3-dev` on some distros
- No flatpak/snap packaging yet
