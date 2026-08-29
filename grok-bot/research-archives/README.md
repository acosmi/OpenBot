# Original release archive

This directory records the identity (size, SHA-256, original URL) of the publicly
distributed Grok Bot 0.18.0 installers used by the reconstruction.

**The installer binaries themselves are not stored in this repository** (OpenBot
first source R116, 2026-08-28). The Git LFS pointers were removed because the
objects had never been pushed to the LFS store (every default `git clone` failed
on the smudge filter), and because what this repository references is the
reconstruction, not the proprietary installer. `artifacts.json` and `SHA256SUMS`
remain the machine-readable identity record; fetch the originals from the URLs
below only if a forensic re-check is ever needed.

## Artifacts

| Platform | Architecture | Bytes | SHA-256 | Original URL |
| --- | --- | ---: | --- | --- |
| macOS | arm64 | 155,793,020 | `a253ccd8aab01e083f9812a0264354c5034d8ba7f0610bbb557e82ae77d203eb` | `https://downloads.cursor.com/grokbot/stable/darwin-arm64/0.18.0/Grok_Bot_0.18.0.dmg` |
| Windows | x64 | 125,825,552 | `464079a15ef5fa8b61ccea8fffcc78f63cfcf6df65fb0ad5e725d8b95f7e437e` | `https://downloads.cursor.com/grokbot/stable/win32-x64/0.18.0/Grok_Bot_0.18.0_Setup.exe` |

The browser download metadata on the archived local copies identified the URLs
above. The macOS checksum also matches the independent pin used by the build
toolchain.

## Fetching and verification

```sh
cd research-archives/original/0.18.0
curl -L -o macos-arm64/Grok_Bot_0.18.0.dmg \
  https://downloads.cursor.com/grokbot/stable/darwin-arm64/0.18.0/Grok_Bot_0.18.0.dmg
curl -L -o windows-x64/Grok_Bot_0.18.0_Setup.exe \
  https://downloads.cursor.com/grokbot/stable/win32-x64/0.18.0/Grok_Bot_0.18.0_Setup.exe
shasum -a 256 -c SHA256SUMS
```

The downloaded files are ignored by Git (`.gitignore` in this directory); do not
re-add them, with or without LFS.

`artifacts.json` is the machine-readable source, size, and digest inventory.
These files are preservation inputs, not reconstructed build outputs.
