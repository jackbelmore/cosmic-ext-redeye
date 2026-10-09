# Patched `cosmic-comp`

redeye drives a real per-channel gamma ramp over
`zwlr_gamma_control_unstable_v1`. Stock Fedora `cosmic-comp` does not
advertise that protocol, so it has to run a patched compositor.

The full tree is the private fork `jackbelmore/cosmic-comp`. One branch per
upstream release, each being the upstream `epoch-X.Y.Z` tag plus a single
commit (rebased from [pop-os/cosmic-comp#2417](https://github.com/pop-os/cosmic-comp/pull/2417)):

| Branch | Base | Patch here |
|---|---|---|
| `gamma-1.9.0` (current) | `epoch-1.9.0` | `gamma-on-1.9.0.patch` |
| `gamma-1.8.0` | `epoch-1.8.0` | `gamma-on-1.8.0.patch` |

Each patch is `git diff epoch-X.Y.Z..gamma-X.Y.Z` from the fork. Keep the
compositor on the same version as the installed `cosmic-session`
(`rpm -q cosmic-session`); a mismatch risks "incompatible cosmic-session and
cosmic-comp versions" and a session that drops back to the greeter.

## Current setup: local build + `/usr/local/bin` symlink

No RPM. `cosmic-session` launches plain `cosmic-comp` using the PATH the
session started with (`/usr/local/bin` ahead of `/usr/bin`), so a symlink in
`/usr/local/bin` wins and leaves the distro package untouched.

```sh
git clone git@github.com:jackbelmore/cosmic-comp.git && cd cosmic-comp
git checkout gamma-1.9.0
cargo build --release                      # ~6 min, toolchain pinned by rust-toolchain.toml
mkdir -p ~/.local/opt/cosmic-comp-fork
cp target/release/cosmic-comp ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork
sudo ln -s ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork /usr/local/bin/cosmic-comp
# then log out and back in
```

Build deps beyond the usual: `libseat-devel libinput-devel`.

Build the compositor and the applet one after the other, not together: both
are memory-hungry on a 16 GB machine.

**Revert:** `sudo rm /usr/local/bin/cosmic-comp`, then log in again. If the
session will not start, Ctrl+Alt+F3, log in on the TTY, run the same command.

### After a Fedora update of `cosmic-comp` / `cosmic-session`

The symlink keeps pointing at the old build, so the running compositor falls
behind the session. Rebase the fork onto the new tag and rebuild:

```sh
git fetch upstream --tags
git checkout -b gamma-X.Y.Z epoch-X.Y.Z
git cherry-pick <gamma commit from the previous branch>
cargo build --release && cp target/release/cosmic-comp ~/.local/opt/cosmic-comp-fork/cosmic-comp-fork
```

## Older route: patched RPM (1.8.0)

`cosmic-comp.spec`, `gamma-on-1.8.0.patch` and `vendor-config-1.8.0.toml` are
the pieces of the patched-RPM build used on 1.8.0. They are kept for
reference and are not used by the current setup.

- `Release: 1.gamma1`, so `rpm -q cosmic-comp` shows whether the patched build is live.
- `debug_package %{nil}`, `debuginfo=0`, `codegen-units=16`: rustc was
  OOM-killed linking with `debuginfo=2` (10.1 GB anon-rss on a 15 GB box).
- Install with `sudo rpm -Uvh --force`, not `dnf`, with `excludepkgs=cosmic-comp`
  in `/etc/dnf/dnf.conf` so an update cannot replace it.
