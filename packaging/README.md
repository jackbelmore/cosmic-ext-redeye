# Modified cosmic-comp

Redeye needs a cosmic-comp that supports `wlr-gamma-control`. The fork
[jackbelmore/cosmic-comp](https://github.com/jackbelmore/cosmic-comp) has one branch per
COSMIC release: the upstream `epoch-X.Y.Z` tag plus Nick Smith's
[#2417](https://github.com/pop-os/cosmic-comp/pull/2417). The latest branch also has a short
README note on top.

| Branch                  | Same change as a patch  |
| ----------------------- | ----------------------- |
| `gamma-1.10.0` (latest) | `gamma-on-1.10.0.patch` |
| `gamma-1.9.0`           | `gamma-on-1.9.0.patch`  |
| `gamma-1.8.0`           | `gamma-on-1.8.0.patch`  |

Use the branch that matches `rpm -q cosmic-session`. A mismatch can stop you logging in.

## Install

```sh
sudo dnf install libseat-devel libinput-devel
git clone -b gamma-1.10.0 https://github.com/jackbelmore/cosmic-comp.git
cd cosmic-comp && cargo build --release      # about 6 minutes
mkdir -p ~/.local/opt/cosmic-comp-fork
cp target/release/cosmic-comp ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork
sudo ln -s ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork /usr/local/bin/cosmic-comp
```

Log out and back in. COSMIC finds `cosmic-comp` through your PATH, where `/usr/local/bin`
comes before `/usr/bin`, so your distro's version is never touched.

To undo it, run `sudo rm /usr/local/bin/cosmic-comp` and log in again. If the desktop
won't load, press Ctrl+Alt+F3 and run it from there.

## After a COSMIC update

Rebuild **before** you log out, or you'll log in to an old cosmic-comp with a newer COSMIC.

```sh
git remote add upstream https://github.com/pop-os/cosmic-comp.git   # first time only
git fetch upstream --tags
git checkout -b gamma-X.Y.Z epoch-X.Y.Z
git cherry-pick epoch-A.B.C..gamma-A.B.C   # A.B.C = the previous branch
cargo build --release
cp target/release/cosmic-comp ~/.local/opt/cosmic-comp-fork/new
mv ~/.local/opt/cosmic-comp-fork/new ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork
```

The copy-then-rename matters: copying straight over the running file fails with
"Text file busy".

## Old RPM build

`cosmic-comp.spec` and `vendor-config-1.8.0.toml` are from an earlier patched RPM of 1.8.0,
kept for reference. It turned off debug info, because linking with it ran out of memory on
16 GB of RAM.
