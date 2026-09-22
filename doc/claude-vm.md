<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Claude Code in a microVM

`scripts/claude-vm.sh` runs Claude Code inside a [smolvm](https://smolmachines.com) microVM
whose whole world is this repository. Two files: the script, and
`scripts/claude-vm/claude.smolfile`, which is what the machine *is*.

```sh
scripts/claude-vm.sh up                 # create + boot + provision, idempotent
scripts/claude-vm.sh claude             # the agent, in /work
scripts/claude-vm.sh yolo               # the same, --dangerously-skip-permissions
scripts/claude-vm.sh shell              # a bash prompt in there
scripts/claude-vm.sh exec cargo test    # one command
scripts/claude-vm.sh upgrade            # pull a newer image into the machine
scripts/claude-vm.sh status | stop | down
```

Needs `smolvm` on `PATH` and `/dev/kvm`. The first `up` pulls the image and takes about two
minutes; every verb after that is a tenth of a second on a running machine.

## Why

Two reasons, and the second is the one that was not expected.

**The blast radius.** `--dangerously-skip-permissions` is a bad idea on a developer's machine
for exactly one reason: what the agent can reach is everything the account can reach — every
other checkout, `~/.ssh`, `~/.aws`, the browser profile, the dotfiles. In here the only host
path that exists is this repository, mounted at `/work`. There is no `$HOME` of yours in the
VM at all. `yolo` is a verb rather than a flag people copy-paste because running it *here* is
the intended use and running it on the host still is not.

**The image is the better test machine.** `ghcr.io/fwilhe2/rust-libreoffice:latest` carries
Rust 1.98.1, LibreOffice 26.8 **and `jing`** on Debian 13. The host has no `jing`, so R2's
schema validation silently skips there; in the VM it runs. `soffice` is on `PATH`, so loops
C, D and E run without `scripts/soffice-docker`. The VM is where `cargo test` says the most.

It has since grown past that, and the rest of CI is now in it too — so the claim is no
longer "the tests" but **every check this repository has**:

| In the image | What it lets the VM run |
|---|---|
| `jing`, `soffice` (Calc + Writer, `-l10n-de`) | R2's schema validation, loops C, D and E |
| `libgtk-4-dev`, `libadwaita-1-dev`, Xvfb, DejaVu + Liberation fonts | `cargo test -p grind-sheet-gtk` / `-p grind-text-gtk`, and `--render-to` |
| `x86_64-pc-windows-msvc`, `cargo-xwin`, `llvm-windres`, Wine 10 (amd64 hosts only) | `scripts/run.sh win32` — link the Windows shell and run it. No `wine32:i386`, which costs nothing: the shell this repository builds is 64-bit |
| `wasm32-unknown-unknown`, `wasm-bindgen-cli` 0.2.127, `binaryen`, node | `ui_web/build.sh` and `ui_web/smoke.sh` |
| `reuse` | `reuse lint`, which CI gates on |
| `cargo-deb`, `cargo-generate-rpm`, `cargo-bloat`, `twiggy` | what `artifacts.yml`'s jobs do after the release build |

The one thing it is still not is CI itself: `artifacts.yml`'s Windows job needs a real
`windows-latest`, and Wine is for looking at a frame, not for signing off on one.

## What it does not protect against

Written down rather than implied, because a sandbox that is trusted for more than it does is
worse than none:

- **Outbound network is unfiltered.** The agent has to reach `api.anthropic.com` and `cargo`
  has to reach `crates.io` and `github.com`. `[network] allow_hosts` exists and is
  deliberately unused — cargo's fetch path resolves more names than a list stays true to, and
  a filter that gets widened after every failure teaches people to widen it without reading.
- **`/work` is the real repository, not a copy.** An agent in there can delete uncommitted
  work. Commit or stash first, the same as ever.
- **The VM holds a copy of your credential.** `~/.claude/.credentials.json` is copied in (or
  `ANTHROPIC_API_KEY` is passed as a smolvm secret *reference*, resolved at launch, never
  persisted). It is a copy: a `/login` on either side does not re-authenticate the other, and
  the script re-syncs it on the next verb because the stamp it compares carries the
  credential's own hash.

## The pieces, and why each is the way it is

| Piece | Why |
|---|---|
| `[dev] volumes = ["./:/work"]` | The one host path in the machine. Read-write on purpose: an agent that cannot save its work is a demo |
| `CARGO_TARGET_DIR=/workspace/target` | The VM's own target directory, on its storage disk. Host and guest share neither glibc nor rustc, and one `target/` serving both is a full rebuild every switch. It survives `stop`/`start`, so the second `cargo test` in there is as fast as the host's |
| `env = ["PATH=…"]` | smolvm hands the workload its own `PATH`, so the image's `/usr/local/cargo/bin` has to be put back. `/etc/profile.d/grind-vm.sh`, written during provisioning, is the same list for login shells — one source, so `exec` and `shell` cannot disagree |
| Provisioning in the script, not `[dev] init` | It has to be re-runnable on a machine that is already up (that is what `creds` is), and it wants an exit code. A stamp in the VM (`/root/.grind-vm-stamp`) holds the state the last run left, so the warm path re-does none of it |
| `git config safe.directory /work` | `/work` is owned by the host user and the VM is root, so git calls it dubious. Scoped to the one mount rather than `*` |
| `DISABLE_AUTOUPDATER=1` | A VM that rewrote its own agent mid-session would not be the machine that was provisioned |
| `IS_SANDBOX=1`, on the `yolo` verb only | Claude Code refuses `--dangerously-skip-permissions` outright when it runs as root, which everything in this VM does. That variable is how the refusal's premise is answered. It is a true statement about this machine and would be a false one almost anywhere else, so it is set on the one command that needs it rather than in the Smolfile |

Changing the Smolfile does not reach a machine that already exists — `down` then `up` for
that. Changing what *provisioning* writes does: bump `PROVISION` in the script and the next
verb re-does it.

## `upgrade`, and why a moving `latest` is safe to depend on

The image reference is a tag, not a digest, and that is deliberate — this is a toolbox, not
an oracle. (The *oracle* is pinned by digest and always will be: loop E's `FLOOR` is a fact
about one `soffice` build, which is what `ci/libreoffice-image` is for. A VM whose `soffice`
drifted under a developer would be a bad place to read that number, and `scripts/soffice-tests.sh`
against the pinned image is still the answer when the number is the question.)

What makes the tag safe is that **it is resolved exactly once, at `machine create`**. The
layers then live on the machine's own storage disk; `start` re-resolves nothing, there is no
`machine update --image`, and `machine prune` frees layers the machine no longer references
rather than fetching new ones. A machine is therefore as reproducible after the tag moves as
before it — the tag moving is invisible until somebody asks for it.

`upgrade` is asking for it: delete, create, pull, provision, and then print the `rustc` and
`soffice` versions the new layers actually contain, because that is the question an upgrade
was run to answer. (`smolvm machine images` is the obvious thing to print and is not worth
it — it truncates the reference to a column width and reports no digest, so two different
builds of `latest` look identical in it.)

The cost is on the VM's own disk and nowhere else: `/workspace/target`, the cargo registry
cache and the credential copy all go, so the first `cargo test` afterwards is a cold build.
`/work` is a host mount and is not in that list, exactly as with `down`.

## Two measured smolvm quirks (1.16.2)

Both cost a debugging session and neither is in `smolvm --help`, so they are written down
here and commented at the code:

1. **`smolvm machine ls` stops a running machine.** The documentation's "Important
   Behaviors" says observational commands leave a running VM alone; `ls` does not. The script
   answers both "does it exist" and "is it running" out of one `machine status`.
2. **Closing a smolvm command's stdout early panics it**, and the VM is left stopped —
   `smolvm machine status … | grep -q` or `| head -1` is enough to do it. Hence no pipes:
   the output is captured into a variable and matched in bash.
