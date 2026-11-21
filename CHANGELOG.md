# Release notes

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
SPDX-License-Identifier: CC-BY-4.0
-->

What changed for the people who use grind, release by release. `## Unreleased` is the draft of
the next release's notes: every change someone using grind would notice adds a line there as it
lands, and the release workflow publishes the section as it stands.

## 0.1.0 — 2026-11-21

### Breaking changes

- The container images' `latest` tag is now the newest release. The newest build of the main
  branch is `dev`.

### Changes

- grind has releases: version numbers, and downloads of the command line and the terminal
  version for Linux, Windows and macOS on the project's releases page.
- On macOS, an installer puts Grind in your own Applications folder, without asking for an
  administrator's password, where Spotlight and Launchpad find it.
- Smaller downloads, and a little faster: every program in the suite is built with whole-program
  optimisation.

### Fixes

- Linux packages built between releases now sort before the release they lead up to, so
  installing the release replaces them.

