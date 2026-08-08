#!/usr/bin/env just --justfile
name := 'wlrix-bg'

rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))
bin-dir := base-dir / 'bin'
# The config file goes in /etc, which is not under the prefix wherever the prefix is -- the same
# reasoning wlrix-greeter's justfile gives for its PAM and greetd files.
etc-dir := absolute_path(clean(rootdir / 'etc'))

bin-src := 'target' / 'release' / name
bin-dst := bin-dir / name

# The system default config. A template rather than a plain file: it names the installed
# wallpaper by absolute path, and a non-/usr prefix has to point at where that actually landed.
# Same @TOKEN@-at-install-time arrangement as wlrix-settings-daemon's activation files.
config-src := 'data' / 'background.toml.in'
config-dst := etc-dir / 'wlrix' / 'background.toml'

# Where `wlrix-assets` installs the wallpapers. **Hand-kept in step with that repo's justfile**,
# which is the one that actually creates the directory; there is no shared variable to point at,
# because the repos build standalone. Getting it wrong is not fatal -- wlrix-bg reports the
# missing file and falls back to the color -- but it is a gray desktop where a scatter should be.
wallpaper-dir := clean(prefix / 'share' / 'wlrix' / 'wallpapers')

default:
  @just --list

release:
  cargo build --release

lint:
  cargo clippy

test:
  cargo test

# Install the wlRIX background and its system default config.
#
# Deliberately does not build: this is normally run as root, and building as root leaves a
# target directory nobody can write to afterwards.
#
#     just release && sudo just install
[doc("Install the background and its default config (build first; run as root)")]
install:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ ! -x '{{bin-src}}' ]; then
    echo "no release build -- run 'just release' first" >&2
    exit 1
  fi
  install -Dm0755 '{{bin-src}}' '{{bin-dst}}'
  echo "installed {{bin-dst}}"

  # /etc belongs to whoever installed the machine. An admin who has edited this file must not
  # have their edit thrown away by a reinstall, so an existing one is left alone and said so --
  # unlike the binary, which is ours and is replaced every time.
  if [ -e '{{config-dst}}' ]; then
    echo "kept {{config-dst}} (already present; not overwritten)"
  else
    # DESTDIR must not leak into the path the file itself carries: a staged install is assembled
    # under rootdir but runs from prefix, so substitute the runtime location.
    install -d "$(dirname '{{config-dst}}')"
    sed 's|@WALLPAPERDIR@|{{wallpaper-dir}}|g' '{{config-src}}' > '{{config-dst}}'
    chmod 0644 '{{config-dst}}'
    echo "installed {{config-dst}}"
  fi
  echo
  echo "The default config points at {{wallpaper-dir}}/scatter.png,"
  echo "which wlrix-assets installs. Without it the desktop comes up plain gray."

# Remove what `install` put down.
#
# The config file is left behind on purpose: it is under /etc, it may have been edited, and an
# uninstall that silently deleted somebody's configuration would be a poor trade for a tidy
# filesystem. `wlrix-session`'s and the greeter's uninstalls leave their configuration too.
[doc("Remove what install put down")]
uninstall:
  #!/usr/bin/env bash
  set -euo pipefail
  rm -f '{{bin-dst}}'
  echo "removed {{bin-dst}}"
  if [ -e '{{config-dst}}' ]; then
    echo "left {{config-dst}} alone -- remove it by hand if you want it gone"
  fi

clean:
  cargo clean
