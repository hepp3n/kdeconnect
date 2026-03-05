# justfile

APPID := "io.github.hepp3n.kdeconnect"
PREFIX := env_var("HOME") / ".local" 
XDG_CONFIG := env_var_or_default("XDG_CONFIG_DIR", "~/.config")

build:
    cargo build --release

build-rel-offline:
    cargo --offline build --release -p kdeconnect-service
    cargo --offline build --release -p cosmic-ext-connect-applet

build-service:
    cargo build --release -p kdeconnect-service

build-applet:
    cargo build --release -p cosmic-ext-connect-applet

run-service:
    cargo run --release -p kdeconnect-service

run-applet:
    cargo run --release -p cosmic-ext-connect-applet

install-bins: build
    install -Dm755 target/release/kdeconnect-service {{PREFIX}}/bin/kdeconnect-service
    install -Dm755 target/release/cosmic-ext-connect-applet {{PREFIX}}/bin/cosmic-ext-connect-applet
    install -Dm755 target/release/cosmic-ext-connect-settings {{PREFIX}}/bin/cosmic-ext-connect-settings
    install -Dm755 target/release/cosmic-ext-connect-sms {{PREFIX}}/bin/cosmic-ext-connect-sms
    @echo "✓ Installed binaries to {{PREFIX}}/bin/"

install-applet-desktop:
    install -Dm644 resources/{{APPID}}.desktop {{PREFIX}}/share/applications/{{APPID}}.desktop
    install -Dm644 resources/{{APPID}}.metainfo.xml {{PREFIX}}/share/metainfo/{{APPID}}.metainfo.xml
    @echo "✓ Installed applet desktop file"

install-systemd: build-service
    mkdir -p {{ XDG_CONFIG }}/systemd/user/
    install -Dm644 kdeconnect-service/kdeconnect.service {{ XDG_CONFIG }}/systemd/user/
    -systemctl --user daemon-reload || echo "⚠️  Could not reload systemd (not in graphical session)"
    @echo "✓ Installed systemd service file"

enable-service:
    systemctl --user enable --now kdeconnect.service
    @echo "✓ Service enabled and started"

status:
    systemctl --user status kdeconnect.service

logs:
    journalctl --user -u kdeconnect.service -f

stop:
    systemctl --user stop kdeconnect.service

restart:
    systemctl --user restart kdeconnect.service

install: install-bins install-systemd install-applet-desktop
    @echo ""
    @echo "✓ Installation complete!"
    @echo ""
    @echo "Next steps:"
    @echo "  1. Log into graphical session (if not already)"
    @echo "  2. Run: just enable-service"
    @echo "  3. Open COSMIC Settings → Desktop → Panel"
    @echo "  4. Click 'Configure Panel Applets'"
    @echo "  5. Add 'KDE Connect' applet"
    @echo ""
    @echo "Or test manually without systemd:"
    @echo "  just run-service"

clean:
    cargo clean

uninstall:
    -systemctl --user stop kdeconnect.service
    -systemctl --user disable kdeconnect.service
    rm -f {{PREFIX}}/bin/kdeconnect-service
    rm -f {{PREFIX}}/bin/cosmic-ext-connect-applet
    rm -f {{PREFIX}}/bin/cosmic-ext-connect-settings
    rm -f {{PREFIX}}/bin/cosmic-ext-connect-sms
    rm -f {{ XDG_CONFIG }}/systemd/user/kdeconnect.service
    rm -f {{PREFIX}}/share/applications/{{APPID}}.desktop
    rm -f {{PREFIX}}/share/metainfo/{{APPID}}.metainfo.xml
    @echo "✓ Uninstalled"
    
