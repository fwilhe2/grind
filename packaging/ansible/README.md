<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# fwilhe2.grind

An Ansible collection with one role, `fwilhe2.grind.grind`, that installs grind's released
Linux packages from its GitHub releases: by default the spreadsheet, the word processor and
the `grind` command line.

It looks up the release, downloads the `.deb` or `.rpm` files, checks each against the sha256
digest GitHub publishes for it, and installs them in one transaction. When the installed
packages are already that release, it downloads nothing and reports no change. A newer release
is an upgrade.

Supported: x86_64 on the Debian family (Debian, Ubuntu), the Red Hat family (Fedora, RHEL and
its rebuilds) and the SUSE family (openSUSE). grind publishes no package for other
architectures or for Arch, and the role fails on them with a message saying so.

## Install

The collection lives in this repository, not on Galaxy:

```sh
ansible-galaxy collection install 'git+https://github.com/fwilhe2/grind.git#/packaging/ansible,main'
```

or in a `requirements.yml`:

```yaml
collections:
  - name: https://github.com/fwilhe2/grind.git#/packaging/ansible
    type: git
    version: main
```

## Use

On this machine, with the playbook the collection ships:

```sh
ansible-playbook fwilhe2.grind.desktop --ask-become-pass
```

In your own playbook:

```yaml
- hosts: workstations
  become: true
  roles:
    - role: fwilhe2.grind.grind
      vars:
        grind_packages: [grind]   # the whole suite: both windows, the TUI and the CLI
```

## Variables

| Variable | Default | |
|---|---|---|
| `grind_version` | `latest` | A release tag such as `v0.2.0`, or `latest` |
| `grind_packages` | `[grind-sheet-gtk, grind-text-gtk, grind-cli]` | Any of `grind-sheet-gtk`, `grind-text-gtk`, `grind-tui`, `grind-cli`, and `grind`, the metapackage for all four |
| `grind_state` | `present` | `absent` removes `grind_packages` |
| `grind_repository` | `fwilhe2/grind` | The GitHub repository whose releases are installed |
| `grind_github_token` | `$GITHUB_TOKEN` on the controller | Optional; anonymous API calls are limited to 60 an hour per address |

`ansible-doc --type role fwilhe2.grind.grind` prints the same from the role's argument
specification, which also checks the values before the role runs.

## Notes

- The packages are not signed. The sha256 digest stands in for the signature, and every
  release file is attested as well: `gh attestation verify <file> -R fwilhe2/grind`.
- The Debian install uses `apt-get` because the `apt` module installs one local `.deb` at a
  time, and the metapackage must go in together with what it depends on.
- Check mode is supported: it looks up the release and reports what it would install.
