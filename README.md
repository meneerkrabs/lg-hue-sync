# lg-hue-sync

[![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/adeze/lg-hue-sync/actions/workflows/ci.yml/badge.svg)](https://github.com/adeze/lg-hue-sync/actions/workflows/ci.yml)
[![webOS](https://img.shields.io/badge/webOS-rooted%205%2F6-blue.svg)](https://www.webosbrew.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Native ambient-light synchronization for rooted LG webOS TVs. A Rust daemon captures the displayed image, derives spatial colours, and streams them to Philips Hue Entertainment, Nanoleaf 4D and Govee RGBIC strips (LAN API razer mode). A responsive LAN dashboard provides pairing, calibration, presets, and independent output controls.

## Features

- Root-only webOS capture through dynamically loaded `libvtcapture`/`dile_vt` APIs.
- Hue Entertainment API v2 over DTLS 1.2 PSK, including gradient member identity.
- Nanoleaf 4D UDP streaming with 40-panel corner, direction, and offset alignment.
- Govee RGBIC output over the LAN API razer/DreamView segment mode (UDP 4003); the strip returns to its own scene whenever sync pauses.
- Independent Hue/Nanoleaf controls; TV sleep/wake following can be automatic or manual.
- Letterbox-aware sampling, HDR compression, OLED black gating, smoothing, and bounded scene changes.
- Responsive dashboard at `http://<tv-ip>:8088/` with system, dark, and light themes.

## Limits and safety

- Root access is required. Rooting can void warranties or render a TV unusable; confirm model and firmware compatibility first.
- DRM-protected native webOS apps play through the secure video path, and the webOS VT driver refuses to capture it by design. The daemon detects that state, stops retrying capture and releases the lights until playback ends; it does not try to capture protected content. External HDMI playback is the path for protected content.
- `libvtcapture` registers on the Luna bus and aborts the process when the executable has no Luna role (install path + `scripts/provision_luna.sh`). `auto` probes it in a child process and falls back to `libdile_vt`; set `"capture_backend": "dile_vt"` to skip the probe.
- Hue decides how physical gradient segments are grouped into Entertainment channels. This project streams the selected area's returned channels; it does not fabricate more.
- LG private capture APIs and community root methods are unsupported by LG and may change with firmware.

Check [cani.rootmy.tv](https://cani.rootmy.tv/) and [webOS Brew](https://github.com/webosbrew) before changing a TV.

## Requirements

- Development host with Rust, Docker, `make`, SSH, and `uv`.
- Rooted webOS TV with root SSH and compatible capture libraries.
- Hue Bridge v2 and/or Nanoleaf 4D on the same LAN.

## Validate and build

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --bin lg-hue-sync -- -D warnings
make build
file target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
```

The canonical target build uses Debian Buster to stay within the LG C1/webOS 6 glibc ceiling. A macOS host build is useful for tests, but cannot run on the TV.

The official community [webosbrew/native-toolchain](https://github.com/webosbrew/native-toolchain) remains the reference webOS SDK. It currently cannot link this Rust dependency graph because its libc lacks `getauxval`, required by Rust's supported ARM standard library and `ring`; see [docs/operations.md](docs/operations.md) for the re-evaluation gate.

## Install and update

First installation, after root SSH already works:

```bash
./scripts/deploy.sh <tv-ip>
```

Existing installation, preserving paired credentials and layout:

```bash
make deploy-bin TV_IP=<tv-ip>
```

Then open `http://<tv-ip>:8088/`, pair devices, choose a Hue Entertainment Area, and save settings. Never copy a populated `config.json` between users or commit it.

Build, rollback, uninstall, and verification details: [docs/operations.md](docs/operations.md).

## Development commands

```bash
cargo run -- pair --bridge <bridge-ip> --output config.json
cargo run -- pair-nanoleaf --ip <controller-ip> --config config.json
cargo run -- sync-hue --config config.json
cargo run -- test-pattern --config config.json
cargo run -- test-nanoleaf --config config.json
cargo run -- run --config config.json
```

Pairing and patterns affect physical devices. Use them only when the owner expects light output.

## Configuration

webOS 5 (e.g. BX/CX, Realtek) is supported through `libdile_vt`, including its 4:2:2 frame format. A Govee-only setup needs no Hue pairing:

```json
{
  "bridge_ip": "", "username": "", "clientkey": "", "entertainment_area_id": "",
  "hue_enabled": false,
  "capture_backend": "auto",
  "govee": { "ip": "192.168.1.50", "segments": 15, "reverse": false, "band_start": 0.0, "band_end": 0.45 }
}
```

Enable the strip's LAN control in the Govee Home app first; verify with `lg-hue-sync test-govee --config config.json`.

Start from [config.example.json](config.example.json), or pair through the dashboard. Runtime configuration contains secrets and stays untracked. On the TV:

```text
/var/home/root/lg-hue-sync/config.json
```

Hue v2 HTTPS calls use a scoped SHA-256 certificate pin established during physical push-link pairing. Status endpoints never return Hue or Nanoleaf secrets.

## Project layout

```text
src/                 daemon, capture, colour, Hue, Nanoleaf, dashboard
webos-app/           optional launcher/dashboard package
scripts/             build, provisioning, deployment, maintenance
docs/                architecture, operations, release runbooks
.agents/skills/      repository-specific agent workflow
```

Useful implementation references:

- [webosbrew/native-toolchain](https://github.com/webosbrew/native-toolchain) — webOS sysroot/toolchain reference.
- [webosbrew/hyperhdr-webos-loader](https://github.com/webosbrew/hyperhdr-webos-loader) — app/service packaging and lifecycle reference.
- [webosbrew/ares-cli-rs](https://github.com/webosbrew/ares-cli-rs) — packaging, install, shell, and transfer tooling.
- [webosbrew/apps-repo](https://github.com/webosbrew/apps-repo) — Homebrew Channel submission format.

## Contributing

This is currently a single-maintainer project. Changes go directly to `main`; no pull request is required unless the maintainer explicitly asks for one. Run the validation commands before pushing.

## License

[MIT](LICENSE)
