<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Homebrew

**Not started; researched 2026-11-27.** What installing grind with `brew` would take, so the
work can be picked up later.

## Two ways into brew

- **Homebrew's own repositories**: `homebrew/core` for command-line programs (formulae),
  `homebrew/cask` for apps. A pull request to Homebrew, reviewed by their maintainers; users
  then run `brew install grind`.
- **A tap of our own**: a public GitHub repository named `homebrew-<name>`, such as
  `fwilhe2/homebrew-grind`, holding Ruby files that describe the packages. Nobody else reviews
  it and nothing is published anywhere else. Users run `brew install fwilhe2/grind/grind`.

## Why not Homebrew's own repositories, yet

- **Notability.** A new formula or cask needs at least 30 forks, 30 watchers or 75 stars, and
  **90 forks, 90 watchers or 225 stars when the repository's owner submits it**. A repository
  under 30 days old is normally not eligible. On 2026-11-27 grind had 8 stars, 0 forks and
  0 watchers.
- **Built from source.** `homebrew/core` builds every formula itself (here with
  `depends_on "rust" => :build` and `cargo install`) and takes no prebuilt binaries. It also
  needs a stable, immutable release tag, which grind's `vX.Y.Z` tags are.
- **The Mac app needs notarizing.** Since 2026-09-01 `homebrew/cask` disables every cask that
  fails Gatekeeper's check, and `--no-quarantine` is gone. `Grind.pkg` and `Grind.dmg` are
  neither signed nor notarized; that takes the Apple Developer Program, $99 a year.

## The licence

Not an obstacle. `homebrew/core` asks for a licence compatible with the Debian Free Software
Guidelines, which AGPL-3.0 is; the formula says `license "AGPL-3.0-or-later"`. A tap of our
own has no requirement at all.

## Software written with AI

Homebrew's acceptance policy says nothing about how a program was written: a package is judged
on its licence, notability, a stable release and building from source. Its AI rules are about
**contributions to Homebrew's own repositories**, and apply only to a pull request there:

- disclose the AI use, and the tool or model, in the pull request;
- review everything generated before asking anyone to review it;
- answer maintainers' questions and review comments yourself, without AI;
- no more than one AI-assisted pull request open at a time, for a non-maintainer;
- no AI as a commit's author, co-author, committer or signatory, so no `Co-Authored-By` or
  `Assisted-by` trailer naming one. The commits in this repository carry one; a pull request
  to Homebrew would need its commits written without it.

None of this applies to a tap of our own.

## The plan, when it is picked up

1. **A tap, `fwilhe2/homebrew-grind`**, created by hand: a public repository and its `Formula/`
   directory.
2. **`Formula/grind.rb`**, installing the release's existing macOS archives,
   `grind-vX.Y.Z-aarch64-apple-darwin.tar.gz` and `grind-vX.Y.Z-x86_64-apple-darwin.tar.gz`,
   each with the checksum from its `.sha256` file: `grind` and `grind-tui`, and a `test do`
   that runs `grind --version`. A formula can also cover Linux (Homebrew on Linux) from the
   `-unknown-linux-musl` archives.
3. **Optionally `Casks/grind.rb`** for `Grind.app` from the `.dmg`. Without notarization
   Gatekeeper still blocks the first launch until the user allows it in System Settings, the
   same as for the `.pkg` downloaded by hand.
4. **A release step** in `release.yml` that writes the new version and checksums into the
   formula and pushes it to the tap. It needs a token with write access to the tap repository
   (a fine-grained token or a deploy key, as a secret); `GITHUB_TOKEN` cannot push to another
   repository. Run a `dry_run` release after adding it.
5. **`homebrew/core` later**, once the notability numbers are met, ideally submitted by someone
   other than the owner (75 stars instead of 225): a from-source formula, sent following the AI
   rules above. The app reaches `homebrew/cask` only once it is notarized.

## Sources

- [Acceptable Formulae](https://docs.brew.sh/Acceptable-Formulae)
- [Package Acceptance Policy](https://docs.brew.sh/Package-Acceptance-Policy) (notability)
- [Acceptable Casks](https://docs.brew.sh/Acceptable-Casks)
- [homebrew-cask's CONTRIBUTING.md](https://github.com/Homebrew/homebrew-cask/blob/main/CONTRIBUTING.md)
  (AI/LLM contributions)
- [Casks deprecated for failing Gatekeeper](https://github.com/orgs/Homebrew/discussions/6482)
