# justfile

APPID      := "io.github.hepp3n.kdeconnect"
PREFIX     := env_var("HOME") / ".local"
XDG_CONFIG := env_var_or_default("XDG_CONFIG_HOME", env_var("HOME") / ".config")

# Build everything
build:
    cargo build --release

# Build Offline
build-rel-offline:
    cargo build --offline --release

# Build individual crates
build-service:
    cargo build --release -p kdeconnect-service

build-applet:
    cargo build --release -p cosmic-ext-connect-applet

# Run without installing
run-service:
    cargo run --release -p kdeconnect-service

run-applet:
    cargo run --release -p cosmic-ext-connect-applet

# Install binaries
install-bins: build
    install -Dm755 target/release/kdeconnect-service          {{PREFIX}}/bin/kdeconnect-service
    install -Dm755 target/release/cosmic-ext-connect-applet   {{PREFIX}}/bin/cosmic-ext-connect-applet
    install -Dm755 target/release/cosmic-ext-connect-settings {{PREFIX}}/bin/cosmic-ext-connect-settings
    install -Dm755 target/release/cosmic-ext-connect-sms      {{PREFIX}}/bin/cosmic-ext-connect-sms

# Install applet desktop entry, icon, and metainfo
install-applet-desktop:
    install -Dm644 resources/{{APPID}}.desktop       {{PREFIX}}/share/applications/{{APPID}}.desktop
    install -Dm644 resources/{{APPID}}.svg           {{PREFIX}}/share/icons/hicolor/scalable/apps/{{APPID}}.svg
    install -Dm644 resources/{{APPID}}.metainfo.xml  {{PREFIX}}/share/metainfo/{{APPID}}.metainfo.xml

# Write D-Bus activation file with correct full path
install-dbus-service:
    mkdir -p {{PREFIX}}/share/dbus-1/services
    printf '[D-BUS Service]\nName={{APPID}}\nExec={{PREFIX}}/bin/kdeconnect-service\n' \
        > {{PREFIX}}/share/dbus-1/services/{{APPID}}.service

# Install XDG autostart entry 
install-autostart:
    install -Dm644 resources/{{APPID}}.daemon.desktop {{XDG_CONFIG}}/autostart/{{APPID}}.daemon.desktop

# Install systemd user service (optional — enables journalctl logging and systemctl control)
install-systemd:
    install -Dm644 kdeconnect-service/kdeconnect.service {{XDG_CONFIG}}/systemd/user/kdeconnect.service
    systemctl --user daemon-reload
    @echo "Run 'just enable-service' to start and enable on login"

# Enable and start the systemd service
enable-service:
    systemctl --user enable --now kdeconnect.service

# Default install — uses D-Bus activation, no systemd required
install: install-bins install-applet-desktop install-dbus-service install-autostart
    @echo ""
    @echo "✓ KDE Connect installed successfully!"
    @echo ""
    @echo "The service will start automatically on next login."
    @echo "To start it now: {{PREFIX}}/bin/kdeconnect-service &"
    @echo ""
    @echo "To add the applet:"
    @echo "  COSMIC Settings → Desktop → Panel → Configure Panel Applets → Add KDE Connect"
    @echo ""
    @echo "Optional — systemd integration:"
    @echo "  just install-systemd && just enable-service"

# Systemd helpers
status:
    systemctl --user status kdeconnect.service

logs:
    journalctl --user -u kdeconnect.service -f

stop:
    systemctl --user stop kdeconnect.service

restart:
    systemctl --user restart kdeconnect.service

clean:
    cargo clean

uninstall:
    -systemctl --user stop kdeconnect.service 2>/dev/null || true
    -systemctl --user disable kdeconnect.service 2>/dev/null || true
    rm -vf {{PREFIX}}/bin/kdeconnect-service
    rm -vf {{PREFIX}}/bin/cosmic-ext-connect-applet
    rm -vf {{PREFIX}}/bin/cosmic-ext-connect-settings
    rm -vf {{PREFIX}}/bin/cosmic-ext-connect-sms
    rm -vf {{XDG_CONFIG}}/systemd/user/kdeconnect.service
    rm -vf {{XDG_CONFIG}}/autostart/{{APPID}}.daemon.desktop
    rm -vf {{PREFIX}}/share/applications/{{APPID}}.desktop
    rm -vf {{PREFIX}}/share/icons/hicolor/scalable/apps/{{APPID}}.svg
    rm -vf {{PREFIX}}/share/metainfo/{{APPID}}.metainfo.xml
    rm -vf {{PREFIX}}/share/dbus-1/services/{{APPID}}.service
    @echo "✓ KDE Connect uninstalled"
