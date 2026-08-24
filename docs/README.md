# dexd documentation

Two sets of pages, for two readers.

[Guides](guides/) are for a venue technician: someone setting up a player, with no Rust
and no video engineering assumed. They run in order, and each says what it needs from the
one before.

[Design](design/) is for a developer new to the project: how the player is built, what
each decision rests on, and what is still open. They can be read in any order; the grouping
below is the order that needs the least backtracking.

The [glossary](glossary.md) defines every term either set uses that is not plain
English, marked by which reader it is for.

## Guides, in order

1. [What dexd is](guides/what-dexd-is.md) — what the player does, what it needs, and what it does not do.
2. [Install dexOS](guides/install-dexos.md) — from a blank SD card to a booted player.
3. [Prepare your video](guides/prepare-video.md) — turning the file the artist gave you into the `.265` and its sidecar.
4. [Configure the exhibit](guides/configure-exhibit.md) — the one file naming the video, the display mode and the connector.
5. [Run, check, troubleshoot](guides/run-check-troubleshoot.md) — starting the player, reading the system log, and going from a symptom to a fix.
6. [Reference](guides/reference.md) — every config key, exit code, file path and refusal message, with its fix.

## Design

### The shape of it

- [Architecture](design/architecture.md) — the layers from decoder to screen, and why the frame never leaves the plane.
- [Building and testing dexd](design/development.md) — the two machines, what each can run, and what a change must satisfy before it lands.

### How the loop holds

- [The endless stream](design/endless-stream.md) — why the video is fed as a stream that never ends rather than looped by the player.
- [The sidecar check](design/sidecar.md) — how a video is bound to its frame rate and checksum, and what happens when they disagree.
- [Startup checks](design/startup-checks.md) — everything dexd refuses to start on, in the order it checks.

### How it stays up

- [Failure handling](design/failure-handling.md) — what answers a fault, in order, and which of them are built.
- [The systemd unit](design/service-unit.md) — what starts the player, restarts it, and keeps the console off the display.

### How it is configured

- [Exhibit config](design/exhibit-config.md) — the file's grammar, the decision tables behind it, and the checks it feeds.

### How it ships

- [Packaging](design/packaging.md) — what the `.deb` contains, what it depends on, and what it does not install.
- [Continuous integration](design/ci.md) — which jobs run when, and what a green run does and does not prove.

### What the claims rest on

- [Measurement record](design/measurements.md) — every number the other pages cite, with the conditions it was taken under.
- [Raspberry Pi media capability](design/pi-capability.md) — what each board can decode and present, with the sources.

### What is not settled

- [Roadmap and open questions](design/roadmap.md) — what is planned, what is undecided, and what was rejected.
