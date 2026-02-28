# justfile

APPID := "io.github.hepp3n.kdeconnect"

# Build everything
build:
    cargo build --release

# Build just the service
build-service:
    cargo build --release -p kdeconnect-service

# Build just the applet
build-applet:
    cargo build --release -p cosmic-connect-applet

# Run the service (for testing)
run-service:
    cargo run --release -p kdeconnect-service

# Run the applet (requires service to be running)
run-applet:
    cargo run --release -p cosmic-connect-applet

# Install all binaries
install-bins:
    install -Dm755 target/release/kdeconnect-service ~/.local/bin/kdeconnect-service
    install -Dm755 target/release/cosmic-connect-applet ~/.local/bin/cosmic-connect-applet
    install -Dm755 target/release/cosmic-connect-sms ~/.local/bin/cosmic-connect-sms
    @echo "✓ Installed binaries to ~/.local/bin/"

# Install applet desktop file
install-applet-desktop:
    install -Dm644 resources/{{APPID}}.desktop ~/.local/share/applications/{{APPID}}.desktop
    install -Dm644 resources/{{APPID}}.metainfo.xml ~/.local/share/metainfo/{{APPID}}.metainfo.xml
    @echo "✓ Installed applet desktop file"

# Install systemd user service
install-systemd:
    mkdir -p ~/.config/systemd/user/
    install -Dm644 kdeconnect-service/kdeconnect.service ~/.config/systemd/user/
    -systemctl --user daemon-reload || echo "⚠️  Could not reload systemd (not in graphical session)"
    @echo "✓ Installed systemd service file"

# Enable and start service
enable-service:
    systemctl --user enable --now kdeconnect.service
    @echo "✓ Service enabled and started"

# Check service status
status:
    systemctl --user status kdeconnect.service

# View service logs
logs:
    journalctl --user -u kdeconnect.service -f

# Stop service
stop:
    systemctl --user stop kdeconnect.service

# Restart service
restart:
    systemctl --user restart kdeconnect.service

# Full install (binaries + systemd + desktop file)
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

# Clean build artifacts
clean:
    cargo clean

# Uninstall everything
uninstall:
    -systemctl --user stop kdeconnect.service
    -systemctl --user disable kdeconnect.service
    rm -f ~/.local/bin/kdeconnect-service
    rm -f ~/.local/bin/cosmic-connect-applet
    rm -f ~/.local/bin/cosmic-connect-sms
    rm -f ~/.config/systemd/user/kdeconnect.service
    rm -f ~/.local/share/applications/{{APPID}}.desktop
    rm -f ~/.local/share/metainfo/{{APPID}}.metainfo.xml
    @echo "✓ Uninstalled"
