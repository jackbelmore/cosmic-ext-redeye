# Redeye

A night light for the [COSMIC](https://system76.com/cosmic) desktop. It makes your screen
warmer at night without turning blacks grey.

Started as a fork of [big-ol-pants/cosmic-ext-redeye](https://github.com/big-ol-pants/cosmic-ext-redeye).

![Rust](https://img.shields.io/badge/Rust-applet-b7410e?logo=rust)
![COSMIC](https://img.shields.io/badge/COSMIC-1.10-48b9c7)
![License](https://img.shields.io/badge/license-MPL--2.0-blue)

## Why

COSMIC doesn't have a night light yet. Most workarounds lay a see-through orange layer over
the screen. A layer can only add colour, so blacks turn a muddy brown.

Redeye changes the screen's colour settings (its gamma) instead. It turns blue down and
leaves black alone.

|             | Orange layer | Redeye      |
| ----------- | ------------ | ----------- |
| Black       | muddy brown  | stays black |
| Screenshots | come out orange | look normal |

COSMIC doesn't let apps change these settings yet. The COSMIC team plans its own night light
for a later release. Until then, Redeye comes in two parts:

- this panel applet
- [a modified cosmic-comp](https://github.com/jackbelmore/cosmic-comp), the part of COSMIC
  that draws your screen, which allows it

## Features

- Day and night settings that fade with the real sunset and sunrise
- Warmth and dim sliders that apply as you drag
- Scroll on the panel icon to adjust it quickly
- Copes with monitors being plugged in, and resets your screen when it closes

## Install (Fedora)

**1. The modified cosmic-comp.** Use the branch that matches your COSMIC version
(`rpm -q cosmic-session`).

```sh
sudo dnf install libseat-devel libinput-devel
git clone -b gamma-1.10.0 https://github.com/jackbelmore/cosmic-comp.git
cd cosmic-comp && cargo build --release
mkdir -p ~/.local/opt/cosmic-comp-fork
cp target/release/cosmic-comp ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork
sudo ln -s ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork /usr/local/bin/cosmic-comp
```

Log out and back in. To undo it, run `sudo rm /usr/local/bin/cosmic-comp` and log in again
(press Ctrl+Alt+F3 to get a terminal if the desktop won't load). After a COSMIC update,
rebuild from the matching branch, see [packaging](./packaging/README.md).

**2. The applet.**

```sh
git clone https://github.com/jackbelmore/cosmic-ext-redeye.git
cd cosmic-ext-redeye && just install-user
```

Add **Cosmic Ext Redeye** to your panel in COSMIC Settings. Close gammastep, wlsunset or
redshift first, as only one app can control the colours at a time.

## Settings

Sunset times come from your location, which starts as London. There's no menu for it yet:

```sh
cd ~/.config/cosmic/io.github.big-ol-pants.CosmicExtRedeye/v1
echo 51.5074 > latitude              # south is negative
echo -0.1278 > longitude             # west is negative
echo 1000 > warmest_temperature_k    # reddest setting, 1000 to 6499
```

Log out and back in afterwards. To see what it's doing: `journalctl --user -f | grep redeye:`

## Don't want to modify COSMIC?

Try [cosmic-nightlight](https://github.com/cosmic-nightlight/cosmic-nightlight). It works on
normal COSMIC, is easy to install, and also keeps blacks black. The trade-off is that the
screen flickers for a second or two whenever the tint changes, and plugging in a monitor
can reset it.

## How I built this

I wanted a night light that didn't make everything look faded. I started from
big-ol-pants' Redeye, which used an orange layer, and worked out it could never look right:
a layer can only add colour, so warming the screen meant greying the blacks.

The proper fix needed COSMIC to allow colour changes. I found Nick Smith's unfinished
attempt to add that, updated it for the current COSMIC, and rebuilt Redeye around it.

I built it with [Claude Code](https://claude.com/claude-code), an AI coding assistant, and
checked the results rather than trusting them:

- read the colour settings back from the graphics card to confirm black stays black
- compared the sunset maths with gammastep in five cities around the world
- fixed a bug that could have turned the screen fully black
- 14 automated tests

The commits are co-authored with Claude, so the history shows how it was made.

## Credits

- [big-ol-pants](https://github.com/big-ol-pants): the original Redeye
- [Nick Smith](https://github.com/nicholaspsmith): gamma control for cosmic-comp
  ([#2417](https://github.com/pop-os/cosmic-comp/pull/2417)), ported from
  [niri](https://github.com/niri-wm/niri) (by phuhl and YaLTeR)
- [Luna Jernberg](https://github.com/bittin): Swedish translation
- Tanner Helland's colour temperature formula, and NOAA's sun position maths

## Licence

The applet is [MPL-2.0](./LICENSE). The cosmic-comp fork is GPL-3.0, like COSMIC.
