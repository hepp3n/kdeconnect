<!--suppress HtmlDeprecatedAttribute -->
<div align="center">
  <h1>⚠️ WORK IN PROGRESS</h1>
  <p>A native KDE Connect implementation for the COSMIC Desktop, written in Rust.<br>
  Many features are working but you may encounter bugs — please report them via <a href="https://github.com/hepp3n/kdeconnect/issues">GitHub Issues</a>.</p>
  <br>
  <img alt="KDE Connect applet on COSMIC desktop environment" src="https://raw.githubusercontent.com/hepp3n/kdeconnect/refs/heads/master/resources/screenshots/applet.png" />
</div>

---

<details>
<summary>✅ Supported Features</summary>

- Device Pairing / Unpairing
- Battery Monitor
- Clipboard Sync (bidirectional)
- Connectivity Report (signal strength / network type)
- Contacts Sync
- Find My Phone
- MPRIS / Media Control (exposed via D-Bus MPRIS2 to COSMIC panel)
- Notifications (receive, action, reply)
- Ping
- Run Commands
- Share Files & URLs (send files, receive files and URLs)
- SMS (conversations, send/receive)
- Plugin Enable / Disable per device
- System Volume (Partial support - May not work on certain devices - Known Mobile App Bug)
- Telephony

</details>

<details>
<summary>🚧 Features Not Yet Supported</summary>

- MousePad / Remote Input
- Presenter Mode
- SFTP / Browse Device
- Virtual Display

</details>

---

## Installing from [COSMIC Flatpak Repository](https://github.com/pop-os/cosmic-flatpak)

```bash
flatpak remote-add --if-not-exists --user cosmic https://apt.pop-os.org/cosmic/cosmic.flatpakrepo
flatpak install --user io.github.hepp3n.kdeconnect
```

---

## Building from Source

### Prerequisites

- [rustup.rs](https://rustup.rs)
- `libxkbcommon-dev` (required on some distros — if the build fails, install this first)
- [`just`](https://github.com/casey/just) command runner

### Quick Start

```bash
git clone https://github.com/hepp3n/kdeconnect.git
cd kdeconnect
just build
just install
```

The service starts automatically on next login via D-Bus activation and XDG autostart.

### Optional: Systemd Integration

For journalctl logging and `systemctl` control instead of D-Bus activation:

```bash
just install-systemd
just enable-service
```

> **Note:** You may need to log out and back in for the applet to appear in the COSMIC panel.
> Once logged back in, go to **COSMIC Settings → Desktop → Panel → Configure Panel Applets** and add KDE Connect.

### Debug Install

Full logging for both the service and panel applet:

```bash
just install-debug
```

- Service logs → `/tmp/kdeconnect-service.log`
- Applet logs → `/tmp/kdeconnect-applet.log`

Restore to standard install with `just install`.

---

## Uninstalling

```bash
just uninstall
```

---

## Building as Flatpak

Requires `flatpak-builder`:

```bash
flatpak-builder --force-clean --user --install-deps-from=flathub --repo=repo --install builddir io.github.hepp3n.kdeconnect.json
```
