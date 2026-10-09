# Redeye

A night light for the [COSMIC](https://system76.com/cosmic) desktop that warms your screen
without washing out the blacks.

![Rust](https://img.shields.io/badge/Rust-applet-b7410e?logo=rust)
![COSMIC](https://img.shields.io/badge/COSMIC-1.9-48b9c7)
![License](https://img.shields.io/badge/license-MPL--2.0-blue)

## Why

COSMIC doesn't ship a night light. The original Redeye faked one by putting a translucent
orange layer over the whole screen. That can only *add* colour, so black turns into a
brownish haze and dark themes look washed out.

Changing the display's gamma ramp fixes this, because it *multiplies* each colour channel
instead:

|                     | Overlay (old)                  | Gamma ramp (now)       |
| ------------------- | ------------------------------ | ---------------------- |
| Maths               | `out = tint + (1 − α) · screen` | `out = gain · screen` |
| Black               | lifted into a haze             | stays at exactly 0     |
| Per-channel control | no, one slope for all three    | yes                    |

The catch is that COSMIC's compositor doesn't support the Wayland protocol for this
(`wlr-gamma-control`). The COSMIC team plans a built-in night light as part of their
colour management work in Epoch 3, and won't take the protocol before then. So the project
has two halves:

- **this applet**, rewritten to drive real gamma ramps
- **[a patched cosmic-comp](https://github.com/jackbelmore/cosmic-comp)** that adds the
  protocol, based on Nick Smith's upstream PR

## Features

- Separate **day and night** settings that fade into each other as the sun sets and rises
  (+3° to −6° elevation, the same window gammastep and redshift use)
- **Warmth** down to 1000 K, plus a **dim** slider
- Move any slider to preview it straight away. **Follow the sun** puts the schedule back
- **Scroll** over the panel icon to nudge the warmth
- Copes with monitors being plugged in and out, and hands the ramp back on exit, so the
  screen is never left tinted

## Install

### 1. The patched compositor

Use the branch that matches your COSMIC version (`rpm -q cosmic-session`). On Fedora:

```sh
sudo dnf install libseat-devel libinput-devel
git clone -b gamma-1.9.0 https://github.com/jackbelmore/cosmic-comp.git
cd cosmic-comp && cargo build --release
mkdir -p ~/.local/opt/cosmic-comp-fork
cp target/release/cosmic-comp ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork
sudo ln -s ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork /usr/local/bin/cosmic-comp
```

Log out and back in. Your distro's `cosmic-comp` is left alone, so undoing it is just
`sudo rm /usr/local/bin/cosmic-comp` and logging in again. More detail, including what to
do after a COSMIC update, is in [packaging/](./packaging/README.md).

### 2. The applet

```sh
git clone https://github.com/jackbelmore/cosmic-ext-redeye.git
cd cosmic-ext-redeye && just install-user
```

Then add **Cosmic Ext Redeye** to your panel in COSMIC Settings.

Gamma control belongs to one app at a time, so stop gammastep, wlsunset or redshift first.

## Alternatives

Running a patched compositor isn't for everyone. If you'd rather not,
[cosmic-nightlight](https://github.com/cosmic-nightlight/cosmic-nightlight) works on stock
COSMIC and is much easier to install. It also uses real gamma, so blacks stay black. It
gets there by briefly switching to another virtual terminal to write the gamma table, so:

|                                  | Redeye                    | cosmic-nightlight            |
| -------------------------------- | ------------------------- | ---------------------------- |
| Stock COSMIC                     | no, needs patched compositor | yes (.deb / flatpak)      |
| Changing the tint                | instant, smooth fades     | 1–2 s flicker each change    |
| Monitor plugged in, screen wakes | re-applied automatically  | can clear the tint           |
| Needs root                       | no                        | a small helper, via polkit   |
| Schedule                         | sun position from your location | time zone or custom times |
| gammastep, wlsunset etc.         | also work with the patched compositor | no          |

Once COSMIC ships its own night light, both of these become stopgaps.

## Configuration

The schedule needs your location. There's no UI for it yet and it defaults to London:

```sh
cd ~/.config/cosmic/io.github.big-ol-pants.CosmicExtRedeye/v1
echo 51.5074 > latitude     # north is positive
echo -0.1278 > longitude    # east is positive
echo 1000 > warmest_temperature_k   # how red 100% warmth goes (1000–6499 K)
```

These are read at startup, so restart the applet after changing them. If something isn't
working, the applet logs to the journal:

```sh
journalctl --user -f | grep redeye:
```

## How I built this

I use COSMIC every day and wanted a proper night light. I started from big-ol-pants'
Redeye and tried to make its colours better with a per-channel colour temperature fit.
That's when I worked out why an overlay could never look right: all three channels share
one slope, so the only way to add warmth is to lift the blacks.

That meant gamma ramps, which meant a compositor that supported them. I found Nick Smith's
closed PR adding the protocol to cosmic-comp, rebased it onto the current COSMIC release,
and run it as my compositor. Then I rewrote the applet around it.

I built it with [Claude Code](https://claude.com/claude-code) as a pair programmer. I
decided what it should do and how, reviewed the changes, and insisted on checking things
on real hardware rather than trusting that they worked:

- Reading the GPU's gamma table back from the kernel to confirm black really is
  `(0, 0, 0)` and the gains match the maths
- Testing the sun-position code against gammastep at five locations, including Auckland,
  Anchorage and Tromsø, to catch longitude and hemisphere sign mistakes
- Catching a bug where a `NaN` would have turned the screen fully black, with no UI left
  to undo it
- 14 unit tests (`cargo test`)

The commits are co-authored with Claude, so the history shows how it was made.

## Credits

- [big-ol-pants](https://github.com/big-ol-pants) for the original Redeye applet
- [Nick Smith](https://github.com/nicholaspsmith) for `wlr-gamma-control` in cosmic-comp
  ([pop-os/cosmic-comp#2417](https://github.com/pop-os/cosmic-comp/pull/2417)), ported from
  [niri](https://github.com/niri-wm/niri)'s implementation by
  [phuhl](https://github.com/phuhl) and [YaLTeR](https://github.com/YaLTeR)
- [Luna Jernberg](https://github.com/bittin) for the Swedish translation
- Colour temperature from Tanner Helland's approximation, sun position from NOAA's
  algorithm (Meeus)

Translations live in [i18n/](./i18n) as [Fluent](https://projectfluent.org/) files. Copy
`en` to your language code to add one.

## License

The applet is [MPL-2.0](./LICENSE). The cosmic-comp fork is GPL-3.0, like upstream.
