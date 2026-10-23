# stratum_sys

System integration layer for Stratum: the managers behind the native shell,
which embeds them directly.

## What's here

| Module | Responsibility |
|---|---|
| `audio` | PulseAudio/PipeWire control, device enumeration, and the native parametric EQ (PipeWire filter-chain modules and links) |
| `bluetooth` | BlueZ over D-Bus: adapter state, device list, pair/connect/trust |
| `wifi` / `net` | NetworkManager state and actions |
| `battery` | UPower/sysfs battery status and power profiles |
| `music` | MPRIS media state and transport control |
| `sysmon` / `gpu` | System performance samples for the dashboard |
| `weather` / `calendar` | Open-Meteo forecasts and ICS calendar feeds for the dashboard |
| `polkit` | polkit authentication agent, including the password-prompt lifecycle |

## Design

The crate is **host-agnostic**. It owns no event loop, no socket, and no UI.
Two conventions make that possible:

- **Queries are pure functions** returning `serde_json::Value` snapshots
  (`audio::devices()`, `wifi::list()`, `battery::status()`, …).
- **Push updates go through a [`Publisher`]** the host supplies. Managers that
  own an event source call it; the host decides what to do with each snapshot.

```rust
use stratum_sys::{managers, Publisher};

let publisher = Publisher::new(|channel, payload| {
    // channel: "audio" | "net" | "bluetooth" | "music" | "battery" | "polkit"
    println!("{}: {}", channel, payload);
});

managers::audio::spawn_monitor(publisher.clone());
```

The polkit agent is the one piece with callback-style interaction rather than
plain snapshots: construct it with a `Publisher`, register it, and answer
prompts with `PolkitAgent::resolve` / `record_failure`.

## Threading

Managers do blocking work (subprocesses, D-Bus calls, PulseAudio). Call them
from worker threads, never from a UI or Wayland event loop. The managers
themselves are `Send + Sync` and safe to drive concurrently; the only shared
mutable state is behind mutexes.

## Notes

- `audio::native_eq` talks to PipeWire directly and runs its own thread; it is
  the most involved module here.
