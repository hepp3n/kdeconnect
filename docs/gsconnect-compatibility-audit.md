# GSConnect Compatibility Audit

Source: `gnome-shell-extension-gsconnect-main.zip`, commit `1a6230dff3247b9e014a9d32f7909b944ecf1097`.

## Implemented or Partially Implemented

| Plugin | GSConnect packet surface | COSMIC status |
| --- | --- | --- |
| Battery | `kdeconnect.battery`, `kdeconnect.battery.request` | Receives phone battery, requests phone state on reconnect, and answers battery requests with local battery/default desktop state. |
| Clipboard | `kdeconnect.clipboard`, `kdeconnect.clipboard.connect` | Basic send/receive. Connect-time initial clipboard behavior is partial. |
| Connectivity Report | `kdeconnect.connectivity_report`, `.request` | Receives cellular state and requests phone state on reconnect. Desktop-side request publishing is intentionally a no-op when no cellular modem state exists. |
| Contacts | `contacts.request_all_uids_timestamps`, `request_vcards_by_uid`, responses | Basic sync and cache path implemented. vCard parsing is simpler than GSConnect. |
| Find My Phone | `kdeconnect.findmyphone.request` | Sends ring requests to the phone and handles incoming requests with a desktop notification plus audible alarm fallback. |
| Mousepad | `mousepad.echo`, `mousepad.request`, `mousepad.keyboardstate` | Packet handling and `ydotool` backend implemented for pointer, buttons, scrolling, typing, special keys, modifiers, and echo acknowledgements. |
| MPRIS | `kdeconnect.mpris`, `.request` | Bidirectional support implemented, including phone MPRIS exposure and album art paths. Needs more edge-case testing against GSConnect tests. |
| Notifications | `notification`, `notification.action`, `notification.reply`, `notification.request` | Receives phone notifications, requests phone notification state on reconnect, forwards action clicks, supports reply through a desktop text prompt, and handles cancel packets. Passive desktop-to-phone mirroring is not implemented. |
| Ping | `kdeconnect.ping` | Send/receive implemented. |
| Presenter | `kdeconnect.presenter` | Packet handling added. Runtime pointer movement requires `ydotool`. |
| Run Commands | `runcommand`, `runcommand.request` | Local command list and remote execution paths implemented. |
| Share | `share.request`, `share.request.update` | File/text/URL paths mostly implemented. Composite transfer/update behavior is incomplete. |
| SFTP | `kdeconnect.sftp`, `kdeconnect.sftp.request` | Browse action sends an SFTP request, handles SFTP/error responses, adds the local key to `ssh-agent` when possible, and opens the returned `sftp://` URI through `gio open`/`xdg-open`. |
| SMS | `sms.messages`, `sms.request`, `sms.request_conversation(s)` | Conversations and legacy-compatible send payload implemented. The GSConnect baseline zip does not advertise attachment request capabilities. |
| System Volume | `systemvolume`, `systemvolume.request` | PulseAudio sink list, volume, and mute handling implemented. |
| Telephony | `telephony`, `telephony.request`, `telephony.request_mute` | Call notifications and media pause/resume are implemented. Volume/microphone policy parity is incomplete. |

## Not Yet Implemented Or Still Partial

| Plugin | Required work |
| --- | --- |
| Notifications | Full GSConnect parity still requires passive desktop notification mirroring to the phone. COSMIC exposes standard notification display through the desktop notification system/portal, but this daemon does not yet implement a notification-bus monitor/proxy like GSConnect's GNOME service component. |
| SFTP | GSConnect-style mounted-directory submenu and explicit unmount action are not implemented in COSMIC UI; the current path opens the device filesystem through the desktop URI handler. |
| Browser Integration | GSConnect ships browser native-host/webextension integration. This repository does not include a webextension/native-host package. |
| Virtual Display | GSConnect zip does not include a virtual display plugin. KDE Connect has a separate protocol surface not covered by this GSConnect baseline. |

## Runtime Dependencies

GSConnect uses GNOME Shell input APIs where available and `ydotool` outside GNOME. The COSMIC implementation now follows the non-GNOME route for Mousepad/Presenter; `/usr/bin/ydotool` is installed on this machine and available to the service.
