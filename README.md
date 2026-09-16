# Cosmic Ext Redeye

A night light applet for the COSMIC desktop, driving a real per-channel gamma ramp.

Warmth and dimming are applied as `out = screen * gain` through
`zwlr_gamma_control_unstable_v1`, so **black stays black**. An earlier version of this
applet tinted with a layer-shell overlay; that can only ever do
`out = tint + (1 - alpha) * screen`, where the slope is one scalar shared by all three
channels, so it could introduce colour only by lifting blacks. That is the washed-out
haze, and it is why the backend was replaced rather than tuned.

## Features

- Day and night presets, fading smoothly as the sun crosses from +3° to −6° — about an
  hour, longer near the solstices. The window matches gammastep and redshift.
- Dragging any slider drops out of the schedule and applies the pair you are dragging, so
  you can see a night setting at midday instead of adjusting it blind. The **Follow the
  sun** toggle puts it back, resuming wherever the sun currently is (expect a jump if you
  do that at midnight — that is the schedule, not a glitch).
- Scroll the panel icon to nudge warmth without opening the popup. Like a drag, this takes
  over from the schedule; the tooltip tells you which mode you are in.

## Requirements

**This applet needs a compositor that implements `zwlr_gamma_control_manager_v1`.** Stock
`cosmic-comp` does not. Upstream has declined the protocol twice, deferring it to a wider
colour-management story, so this is not a matter of waiting for a release.

Check with:

```sh
wayland-info | grep zwlr_gamma_control_manager_v1
```

If it is absent the applet will say so in its popup and do nothing else. Getting it
present means running a patched `cosmic-comp` carrying
[PR #2417](https://github.com/pop-os/cosmic-comp/pull/2417). If you build one, pin it, or
the next distro upgrade will silently replace it and the applet will go inert:

```sh
# /etc/dnf/dnf.conf, under [main]
excludepkgs=cosmic-comp
```

Gamma control is **exclusive per output**, so gammastep, wlsunset or redshift cannot run
alongside this. Whichever binds second is refused — gammastep words that refusal
misleadingly as "Zero outputs support gamma adjustment".

## Configuration

Slider positions and presets are stored per key under
`~/.config/cosmic/io.github.big-ol-pants.CosmicExtRedeye/v1/`.

Location has no UI. Set it by editing two files there, which default to London:

```sh
echo 51.5074 > ~/.config/cosmic/io.github.big-ol-pants.CosmicExtRedeye/v1/latitude
echo -- -0.1278 > ~/.config/cosmic/io.github.big-ol-pants.CosmicExtRedeye/v1/longitude
```

Latitude is north-positive, longitude **east**-positive. They are read at startup only, so
restart the applet after changing them — the applet never writes them back, so an edit
made while it is running will not be overwritten.

The reddest the "Warmth" slider goes at 100% also has no UI. It defaults to 1000 K:

```sh
echo 1000 > ~/.config/cosmic/io.github.big-ol-pants.CosmicExtRedeye/v1/warmest_temperature_k
```

Lower is redder. Values are clamped to a sensible range -- at least 1000 K, below
which the colour math has already saturated and going lower does nothing extra,
and strictly below 6500 K, the neutral point the slider fades from, since anything
at or above that flattens or inverts the slider. Like latitude and longitude, this
is read at startup only and never written back, so restart the applet after
changing it.

Diagnostics go to the panel's journal, which inside COSMIC is the only place to see them:

```sh
journalctl --user -f | grep redeye:
```

## Installation

A [justfile](./justfile) is included by default for the [casey/just][just] command runner.

- `just` builds the application with the default `just build-release` recipe
- `just run` builds and runs the application
- `just install` installs the project into the system
- `just install-user` installs the applet into `~/.local` for current-user testing
- `just vendor` creates a vendored tarball
- `just build-vendored` compiles with vendored dependencies from that tarball
- `just check` runs clippy on the project to check for linter warnings
- `just check-json` can be used by IDEs that support LSP

## COSMIC Panel

Running the binary directly starts the applet surface as a small transparent window. For panel
placement, install the desktop entry and launch it through COSMIC Panel:

```sh
just install-user
```

Then add `io.github.big-ol-pants.CosmicExtRedeye` to the upper-right panel plugin list through COSMIC
Settings, or by editing the second list in:

```sh
~/.config/cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings
```

## Translators

[Fluent][fluent] is used for localization of the software. Fluent's translation files are found in the [i18n directory](./i18n). New translations may copy the [English (en) localization](./i18n/en) of the project, rename `en` to the desired [ISO 639-1 language code][iso-codes], and then translations can be provided for each [message identifier][fluent-guide]. If no translation is necessary, the message may be omitted.

## Packaging

If packaging for a Linux distribution, vendor dependencies locally with the `vendor` rule, and build with the vendored sources using the `build-vendored` rule. When installing files, use the `rootdir` and `prefix` variables to change installation paths.

```sh
just vendor
just build-vendored
just rootdir=debian/cosmic-ext-redeye prefix=/usr install
```

It is recommended to build a source tarball with the vendored dependencies, which can typically be done by running `just vendor` on the host system before it enters the build environment.

## Developers

Developers should install [rustup][rustup] and configure their editor to use [rust-analyzer][rust-analyzer]. To improve compilation times, disable LTO in the release profile, install the [mold][mold] linker, and configure [sccache][sccache] for use with Rust. The [mold][mold] linker will only improve link times if LTO is disabled.

[fluent]: https://projectfluent.org/
[fluent-guide]: https://projectfluent.org/fluent/guide/hello.html
[iso-codes]: https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes
[just]: https://github.com/casey/just
[rustup]: https://rustup.rs/
[rust-analyzer]: https://rust-analyzer.github.io/
[mold]: https://github.com/rui314/mold
[sccache]: https://github.com/mozilla/sccache
