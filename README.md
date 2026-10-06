# grind

# What is this?

grind is an office suite written from scratch. It is 100% written using AI agents. Uses OpenDocument (odf) as its native format.

# Why this project exists?

I'm a software developer and a long-time Linux user. I love and appreciate the free and open source software ecosystem, and I've made an effort not to be trapped in non-free file formats for a long time.

But I've also gotten frustrated with the LibreOffice user exprience. Don't get me wrong, the project has done so much for the free desktop, and it is great software that solves real problems. But the UX shows its age, and the community overall seems not to welcome change. I can never remember how to make Excel-like tables in Calc, and that is not my fault, this is a common operationn that should be simple, but Calc makes it hard for some reason.

I know, many people will close this repo once they discover it is 'slop'. I don't want to argue if AI agents can possible be used in ethical ways, but this human written readme should tell you something: This is a passion project. I would not have the time and energy to do this in any different way, an office suite is just way too complex for that.

# Project Status

No release exists yet. A spredsheet and a word processor are included, the spradsheet is more complete, both are servicable for my own personal use.

# Approach

This project has clients for Linux, Windows, Mac, the Browser (using WebAssembly), a TUI and a CLI. It does purposefully not use any cross-platform toolkit, it exists as a proof of an idea: AI Agents enable a new way of building software, native for each platform. This was possible before but almost no one did it for economical reasons. Not enough benefit for the cost. With AI Agents, this calcualation changes.

## Human and AI Authorship

This readme is fully human written. Source code, docs in the `docs` directory and all other content are AI authored.

# Novel Features

1\. View source

In spreadsheets, it is often not obvious which cells contain data and which contain code (formulas). Authors may decide to use styles to signal this, but that is not more than a convention. Grind has a plain-text projection of its OpenDocument format which make it simple to review and audit spreadsheets.

2\. Formulas in plain English

Grind can show spreadsheet formulas in a way that is more self documenting compared to other spreadsheet software.

3\. A domain specific language to generate documents

Based on [rhai](https://rhai.rs/), an embeddable scripting language you can generate spreadsheets based on dynamic data without the need for a turing complete language. Grind does not have any macro support and never will, but many macro use-cases might be replaced with rhai scripts.

4\. A CLI that can do everything

The cli is a first class citizen, no matter if it is used by a human, in shell scripts or by AI Agents.

# License

Licensed under GNU Affero General Public License v3 or later. Full text in [LICENSE](./LICENSE).

# Trademarks

Product names and trademarks mentioned in this README belong to their respective owners. This project is not affiliated with or endorsed by them.
