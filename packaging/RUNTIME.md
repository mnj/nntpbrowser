# Running NNTPBrowser

Linux packages target amd64 or arm64, with the portable binary built on Ubuntu
22.04. Install Debian packages using `sudo apt install ./nntpbrowser_VERSION_ARCH.deb`;
RPM packages use your distribution's package manager. AppImages must be made
executable before launch. Flatpak packages require their declared Flathub runtime.
For tar archives, unpack and run `bin/nntpbrowser`; the included desktop entry
and icon are not installed automatically. The GUI requires X11 or Wayland and
working OpenGL/EGL drivers. Portable tar archives require system libc, libgcc,
libX11, libxkbcommon, libwayland-client, libGL and libEGL.

Windows packages support x64, x86 and ARM64. MSI installs a Start menu shortcut;
portable ZIPs can be extracted anywhere writable. Run `nntpbrowser.exe` for the
GUI or `nntpbrowser-cli.exe --help` for terminal commands. Installers and
executables are unsigned. The Visual C++ runtime is linked statically.

The macOS universal DMG supports Apple Silicon and Intel on macOS 12 or later.
Drag NNTPBrowser into Applications. The app is ad-hoc signed, without Apple
notarization; macOS may require Privacy & Security → Open Anyway. The CLI ZIP
contains the same binary for terminal use.

FreeBSD archives target FreeBSD 14.3+ amd64. Install libX11, libXcursor,
libXrandr, libXi, libxcb, libxkbcommon, wayland and mesa-libs as needed for your
desktop, then run `bin/nntpbrowser`.

`ffmpeg` is optional for video posters; `unrar` or `unar` is optional for RAR
extraction. These tools and graphics drivers are not bundled. Tools must be on
the application's inherited PATH; Finder-launched macOS applications may not
inherit Homebrew's terminal PATH. Launch from a suitably configured terminal
when using those tools. Flatpak uses its sandbox PATH and cannot automatically
use these tools from the host. Gallery previews stay in memory; saving or queueing a
download is explicit. Settings and caches use platform per-user directories;
the data directory is configurable in Settings.

The application's MIT license, dependency audit, generated third-party notices
and exact MPL-covered `option-ext` source accompany each native package.
Dependencies, fonts and native code retain their own terms. Consult
`third-party/INDEX.md` and copied upstream texts. AppImage additionally includes
its runtime's notices and corresponding source/build materials. The dependency
audit is not legal certification.
