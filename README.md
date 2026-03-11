<!--suppress HtmlDeprecatedAttribute -->
<div align="center">
  <h1>WORK IN PROGRESS</h1>
  <br>
  <img alt="KDE Connect applet on COSMIC desktop environment" src="https://raw.githubusercontent.com/hepp3n/kdeconnect/refs/heads/master/resources/screenshots/applet.png" />
</div>

# Testing on COSMIC Desktop

For testing COSMIC desktop applet, you can build it with help of justfile.

# Cloning repository

`git clone https://github.com/hepp3n/kdeconnect.git`

# Entering directory

`cd kdeconnect`

# Building

`just build`

# Installing

`just install`

# Enable kdeconnect-service

`just enable-service`

**May need to reboot to get applet to show on panel**

# Uninstalling

`just uninstall`

Make sure you have [rustup.rs](https://rustup.rs) installed on your system.

You might need also `libxkbcommon-dev` dependency. If it won't build, please create an issue.

# Building as Flatpak

You can also build this applet as flatpak package. You need to install `flatpak-builder` and then run this command:

`flatpak-builder --force-clean --user --install-deps-from=flathub --repo=repo --install builddir io.github.hepp3n.kdeconnect.json`
