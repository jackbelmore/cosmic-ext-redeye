# Patched `cosmic-comp` packaging

redeye drives a real per-channel gamma ramp over
`zwlr_gamma_control_unstable_v1`. Stock Fedora `cosmic-comp` does not
advertise that protocol, so the compositor has to be rebuilt with it.

These are the non-regenerable pieces of that build.

| File | What it is |
|---|---|
| `gamma-on-1.8.0.patch` | `wlr-gamma-control-unstable-v1`, rebased from [pop-os/cosmic-comp#2417](https://github.com/pop-os/cosmic-comp/pull/2417) onto the `epoch-1.8.0` tag |
| `cosmic-comp.spec` | Fedora spec, edited to apply the patch and to work around an OOM |
| `vendor-config-1.8.0.toml` | `cargo vendor` config the spec expects as `Source2` |

The full compositor tree lives in the private fork
`jackbelmore/cosmic-comp`, branch `gamma-1.8.0`; the patch here is just
`git diff epoch-1.8.0..gamma-1.8.0` from it.

## Spec changes vs. Fedora's

- `Release: 1.gamma1` — so `rpm -q cosmic-comp` always tells you whether the
  patched or the stock build is live.
- `Patch0: gamma-on-1.8.0.patch`.
- `debug_package %{nil}`, `debuginfo=0`, `codegen-units=16` — rustc was
  OOM-killed linking the binary (10.1 GB anon-rss on a 15 GB box).
  `debuginfo=2` is what inflates it, and raising codegen-units lowers peak
  memory per unit and restores parallelism.

## Rebuilding

`Source0`/`Source1` are large and regenerable, so they are not stored here:

```sh
spectool -g -R packaging/cosmic-comp.spec     # fetches cosmic-comp-1.8.0.tar.gz
# vendor-1.8.0.tar.gz: clone upstream at the spec's %commit, then
#   cargo vendor > vendor-config-1.8.0.toml && tar -pczf vendor-1.8.0.tar.gz vendor
cp packaging/gamma-on-1.8.0.patch packaging/vendor-config-1.8.0.toml ~/rpmbuild/SOURCES/
rpmbuild -ba packaging/cosmic-comp.spec
```

Install with `sudo rpm -Uvh --force`, not `dnf`: `/etc/dnf/dnf.conf` carries
`excludepkgs=cosmic-comp` so a system update cannot silently replace the
patched build with Fedora's.

## If it will not boot

`~/proj/cosmic-comp-rollback/` has the stock RPM cached plus `ROLLBACK.sh`,
runnable from a TTY. See the README there.
