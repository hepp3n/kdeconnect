# GSConnect Compatibility Audit

Source: `gnome-shell-extension-gsconnect-main.zip`, commit `1a6230dff3247b9e014a9d32f7909b944ecf1097`.

## Implemented or Partially Implemented

| Plugin | GSConnect packet surface | COSMIC status |
| --- | --- | --- |
| Battery | `kdeconnect.battery`, `kdeconnect.battery.request` | Receives phone battery. Desktop battery publishing/request response still incomplete. |
| Clipboard | `kdeconnect.clipboard`, `kdeconnect.clipboard.connect` | Basic send/receive. Connect-time initial clipboard behavior is partial. |
| Connectivity Report | `kdeconnect.connectivity_report`, `.request` | Receives cellular state. Request handling is partial. |
| Contacts | `contacts.request_all_uids_timestamps`, `request_vcards_by_uid`, responses | Basic sync and cache path implemented. vCard parsing is simpler than GSConnect. |
| Find My Phone | `kdeconnect.findmyphone.request` | Sends ring request. Incoming ring support is not implemented. |
| Mousepad | `mousepad.echo`, `mousepad.request`, `mousepad.keyboardstate` | Packet handling added. Runtime input requires `ydotool`. |
| MPRIS | `kdeconnect.mpris`, `.request` | Bidirectional support implemented, including phone MPRIS exposure and album art paths. Needs more edge-case testing against GSConnect tests. |
| Notifications | `notification`, `notification.action`, `notification.reply`, `notification.request` | Receives phone notifications. Desktop notification mirroring, action activation, reply, and cancellation are incomplete. |
| Ping | `kdeconnect.ping` | Send/receive implemented. |
| Presenter | `kdeconnect.presenter` | Packet handling added. Runtime pointer movement requires `ydotool`. |
| Run Commands | `runcommand`, `runcommand.request` | Local command list and remote execution paths implemented. |
| Share | `share.request`, `share.request.update` | File/text/URL paths mostly implemented. Composite transfer/update behavior is incomplete. |
| SMS | `sms.messages`, `sms.request`, `sms.request_conversation(s)` | Conversations and legacy-compatible send payload implemented. Attachments are incomplete. |
| System Volume | `systemvolume`, `systemvolume.request` | PulseAudio sink list, volume, and mute handling implemented. |
| Telephony | `telephony`, `telephony.request`, `telephony.request_mute` | Call notifications and media pause/resume are implemented. Volume/microphone policy parity is incomplete. |

## Not Yet Implemented

| Plugin | Required work |
| --- | --- |
| SFTP | Send `kdeconnect.sftp.request`, mount returned `sftp://host:port/` via GVfs/ssh, handle `errorMessage`, unmount and expose browse UI. |
| Virtual Display | GSConnect zip does not include a virtual display plugin. KDE Connect has a separate protocol surface not covered by this GSConnect baseline. |

## Runtime Dependencies

GSConnect uses GNOME Shell input APIs where available and `ydotool` outside GNOME. The COSMIC implementation now follows the non-GNOME route for Mousepad/Presenter; install and configure `ydotool` for those features to actually move the pointer or type keys.
