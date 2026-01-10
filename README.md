# Testing on COSMIC Desktop

For testing COSMIC desktop applet, you can build it with help of Makefile.

# Cloning repository

`git clone https://github.com/hepp3n/kdeconnect.git`

# Entering directory

`cd kdeconnect`

# Building

`make build`

# Installing

`sudo make install`

# Uninstalling

`sudo make uninstall`

Make sure you have [rustup.rs](https://rustup.rs) installed on your system.

You might need also `libxkbcommon-dev` dependency. If it won't build, please create an issue.

# Building as Flatpak

You can also build this applet as flatpak package. You need to install `flatpak-builder` and then run this command:

`flatpak-builder --force-clean --user --install-deps-from=flathub --repo=repo --install builddir dev.heppen.CosmicExtConnect.json`
