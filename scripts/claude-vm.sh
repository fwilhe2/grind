#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Run Claude Code in a microVM that can see this repository and nothing else of yours.
#
#   scripts/claude-vm.sh up                 # create + boot + provision (idempotent)
#   scripts/claude-vm.sh claude             # the agent, interactively, in /work
#   scripts/claude-vm.sh yolo               # the same with --dangerously-skip-permissions
#   scripts/claude-vm.sh claude -p 'hi'     # arguments are passed straight through
#   scripts/claude-vm.sh shell              # a bash prompt in the VM
#   scripts/claude-vm.sh exec cargo test    # one command, non-interactive
#   scripts/claude-vm.sh creds              # re-copy the host credential (after a re-login)
#   scripts/claude-vm.sh upgrade            # pull a newer image: delete, re-create, re-provision
#   scripts/claude-vm.sh status | stop | down
#
# The machine is scripts/claude-vm/claude.smolfile: Rust + LibreOffice + jing on Debian 13,
# networking on, this directory bind-mounted read-write at /work and *nothing else of the
# host's filesystem mounted at all*. No ~/.ssh, no ~/.aws, no ~/.config, no other checkout.
#
# That boundary is what `yolo` is for. `--dangerously-skip-permissions` is a bad idea on a
# developer's machine for one reason — the blast radius is everything that account can reach
# — and this shrinks the radius to one directory that is already in git. Running it in here
# is the intended use; running it on the host is still not.
#
# What the VM is *not* protection against: it has unfiltered outbound network (the agent has
# to reach api.anthropic.com, cargo has to reach crates.io), it holds a copy of your Claude
# credential, and /work is the real repository rather than a copy — an agent in here can
# delete your uncommitted work. Commit or stash before you turn it loose, the same as ever.
#
# It is also the only machine here that can run the whole test suite: `jing` is in the image,
# so R2's schema validation actually runs, and `soffice` is too, so loops C, D and E do. The
# image has since grown the rest of CI as well — GTK and Xvfb, Wine and cargo-xwin, the wasm
# target and a matched wasm-bindgen, `reuse`, the two packagers — so every check this
# repository has now runs in here. `upgrade` is how that reached an existing machine:
# `latest` is resolved once, at create, so the tag moving does not touch a VM that exists.
# Its `cargo` is Debian's glibc and its target directory is the VM's own (/workspace/target,
# on the storage disk) — the host's target/ is never touched, and neither build invalidates
# the other.
#
#   GRIND_VM_NAME   the machine's name (default: grind-claude)
#
# Needs smolvm (https://smolmachines.com) and /dev/kvm.

set -euo pipefail
cd "$(dirname "$0")/.."

NAME="${GRIND_VM_NAME:-grind-claude}"
SMOLFILE="scripts/claude-vm/claude.smolfile"

die() { printf '%s\n' "$*" >&2; exit 1; }
note() { printf '\033[2m%s\033[0m\n' "$*" >&2; }

command -v smolvm >/dev/null || die "smolvm is not on PATH — see https://smolmachines.com"
[ -e /dev/kvm ] || die "/dev/kvm is missing — smolvm needs KVM on Linux"

# A credential reaches the VM one of two ways, and never as a mounted host directory: an
# ANTHROPIC_API_KEY in the host environment becomes a *reference* resolved at launch (smolvm
# persists the reference, never the value), otherwise the OAuth credential is copied in once
# and lives on the VM's own disk. The copy is a copy: refreshing it in here does not refresh
# the host's, and `creds` is how you re-sync after a re-login on either side.
secret_args=()
if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
    secret_args=(--secret-env "ANTHROPIC_API_KEY=ANTHROPIC_API_KEY")
fi

# Both questions come out of one `machine status`, whose whole output is captured rather
# than piped. Two measured quirks of smolvm 1.16.2 are behind that, and neither is
# documented: `machine ls` **stops a running machine** (its own documentation says
# observational commands do not), and any smolvm command whose stdout is closed early — the
# `| grep -q` or `| head -1` this would otherwise be written with — panics on the broken
# pipe and leaves the VM stopped. So: no `ls`, and no pipe.
state() { smolvm machine status --name "$NAME" 2>/dev/null || true; }
running() { case "$(state)" in *running*) return 0 ;; *) return 1 ;; esac; }
exists()  { [ -n "$(state)" ]; }

# One `machine exec`, with the secret reference attached if there is one. Everything below
# goes through this so there is exactly one place that knows how a command enters the VM.
vm() { smolvm machine exec --name "$NAME" "${secret_args[@]}" -- "$@"; }

# The same, with a terminal — the agent and the shell both want one.
vm_tty() { smolvm machine exec -it --name "$NAME" "${secret_args[@]}" -- "$@"; }

copy_credentials() {
    if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
        note "credential: ANTHROPIC_API_KEY, passed by reference at launch"
        return
    fi
    local src="$HOME/.claude/.credentials.json"
    if [ ! -f "$src" ]; then
        note "no $src on the host — run \`claude\` inside the VM once and log in there"
        return
    fi
    vm mkdir -p /root/.claude
    smolvm machine cp "$src" "$NAME:/root/.claude/.credentials.json" >/dev/null
    vm chmod 600 /root/.claude/.credentials.json
    note "credential: copied $src into the VM (it is a copy — re-run \`creds\` after a re-login)"
}

# What a provisioned machine has in it, as one string: the agent, the profile, the git
# identity and *which* credential. Comparing it against the stamp the last run left is what
# keeps `scripts/claude-vm.sh exec …` from re-doing all of that — and re-copying a credential
# — on every single command. The credential's own hash is in it, so a `claude /login` on the
# host re-syncs the VM on the next verb with nothing to remember.
STAMP=/root/.grind-vm-stamp
# Bump PROVISION when anything provision() writes changes shape, so an existing machine
# re-provisions on its next verb instead of keeping what an older copy of this script left.
PROVISION=1
want_state() {
    if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
        printf 'v%s:api-key' "$PROVISION"
    elif [ -f "$HOME/.claude/.credentials.json" ]; then
        printf 'v%s:%s' "$PROVISION" "$(sha256sum "$HOME/.claude/.credentials.json" | cut -d' ' -f1)"
    else
        printf 'v%s:none' "$PROVISION"
    fi
}

provision() {
    # Idempotent, and deliberately not the Smolfile's `[dev] init`: it has to be re-runnable
    # on a machine that is already up (that is what `creds` is), and an install that is
    # skipped wants to say so.
    if vm test -x /root/.local/bin/claude 2>/dev/null; then
        note "claude: $(vm claude --version 2>/dev/null | tr -d '\r')"
    else
        note "installing Claude Code in the VM…"
        vm bash -lc 'curl -fsSL https://claude.ai/install.sh | bash' >/dev/null
        note "claude: $(vm claude --version 2>/dev/null | tr -d '\r')"
    fi

    # A login shell reads /etc/profile and sets its own PATH, so the Smolfile's — which is
    # what every non-interactive `exec` gets — has to be put back for `shell` and for
    # anything the agent runs through bash. One file, so the two paths agree.
    vm bash -c 'cat >/etc/profile.d/grind-vm.sh <<EOF
export PATH=/root/.local/bin:/usr/local/cargo/bin:\$PATH
export CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup
export CARGO_TARGET_DIR=/workspace/target
export DISABLE_AUTOUPDATER=1
EOF'

    # /work is owned by the host user and the VM is root, so git calls it dubious. It is
    # scoped to the one mount rather than to '*': the VM has no other checkout in it, and a
    # blanket rule would still be there if someone later mounts one.
    vm git config --global --replace-all safe.directory /work

    # Commits made in here should be yours rather than "root@localhost". Read from the host's
    # git config, which is the one piece of host configuration this VM takes — and it takes
    # the value, not the file.
    local gname gmail
    gname=$(git config --get user.name || true)
    gmail=$(git config --get user.email || true)
    [ -n "$gname" ] && vm git config --global user.name  "$gname"
    [ -n "$gmail" ] && vm git config --global user.email "$gmail"

    copy_credentials
    vm bash -c "printf %s '$(want_state)' > $STAMP"
}

up() {
    if ! exists; then
        note "creating $NAME from $SMOLFILE (the image pull is a few minutes, once)"
        smolvm machine create --name "$NAME" --smolfile "$SMOLFILE" "${secret_args[@]}" >&2
    fi
    if ! running; then
        note "starting $NAME…"
        smolvm machine start --name "$NAME" >&2
    fi
    local have
    have=$(vm cat "$STAMP" 2>/dev/null | tr -d '\r\n' || true)
    [ "$have" = "$(want_state)" ] || provision
}

# The only way a newer image reaches this machine, and the reason it is a verb rather than a
# sentence in the documentation: the Smolfile's `latest` is resolved once, at `create`, and
# the layers then live on the machine's own storage disk. `start` re-resolves nothing, there
# is no `machine update --image`, and `machine prune` frees layers *that machine* no longer
# references rather than fetching new ones. So an upgrade is a delete and a create — which
# is also why `up` alone is not enough and why a moving `latest` is safe to depend on.
#
# What it costs is all on the VM's own disk: /workspace/target, the cargo registry cache and
# the credential copy go, so the next `cargo test` in there is a cold build. /work is a host
# mount and is not in that list — the repository is untouched, exactly as with `down`.
# Anything else you left inside the VM is not, so this is the one verb worth reading twice.
upgrade() {
    if exists; then
        note "deleting $NAME — its target directory and cargo cache go with it (/work does not)"
        smolvm machine delete --name "$NAME" -f >&2
    fi
    up
    # What the pull actually landed. `machine images` is the obvious thing to print here and
    # is not worth it: it truncates the reference to a column width and reports no digest, so
    # two different builds of `latest` look identical in it. The machine itself is asked
    # instead, and the two versions an upgrade is usually chasing are the answer. Both pipes
    # are inside the guest or read to EOF — see `state` for why that distinction matters.
    local have
    have=$(vm bash -lc 'rustc --version; soffice --version 2>/dev/null | head -1' 2>/dev/null \
           | tr -d '\r' | tr '\n' ' ' || true)
    note "image: ${have:-could not be read back}"
}

case "${1:-claude}" in
    up)     up ;;
    upgrade) upgrade ;;
    creds)  up >/dev/null; copy_credentials ;;
    claude) up >/dev/null; shift || true; vm_tty claude "$@" ;;
    # The reason this file exists, spelled as its own verb so it is chosen rather than
    # copy-pasted: no permission prompts, inside a machine whose whole world is /work.
    #
    # IS_SANDBOX=1 is load-bearing and belongs to this verb rather than to the machine:
    # Claude Code refuses `--dangerously-skip-permissions` outright when it is running as
    # root, which every process in this VM is, and that variable is the documented way to
    # say the refusal's premise does not hold here. It is a true statement about this VM and
    # would be a false one almost anywhere else, so it is set on the one command that needs
    # it and not in the Smolfile's env.
    yolo)   up >/dev/null; shift || true
            vm_tty env IS_SANDBOX=1 claude --dangerously-skip-permissions "$@" ;;
    shell)  up >/dev/null; vm_tty bash -l ;;
    exec)   up >/dev/null; shift; [ $# -gt 0 ] || die "exec needs a command"; vm "$@" ;;
    status) smolvm machine status --name "$NAME" ;;
    stop)   smolvm machine stop --name "$NAME" ;;
    # Deletes the VM and everything on its disk — the cargo cache, the target directory and
    # the credential copy. /work is a host mount, so the repository is not in that list.
    down)   smolvm machine delete --name "$NAME" -f ;;
    -h|--help|help) sed -n '5,44p' "$0" ;;   # the header block, down to the `Needs smolvm` line
    *)      die "unknown verb: $1 (try: up, claude, yolo, shell, exec, creds, upgrade, status, stop, down)" ;;
esac
