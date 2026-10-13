<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/media/logo-dark.svg">
    <img src="docs/media/logo-light.svg" width="150" alt="Seamtype: A, 아, あ">
  </picture>
</p>

<h1 align="center">seamtype-rust</h1>

<p align="center"><b>A Windows input method that needs no Korean/English toggle</b></p>

<p align="center"><a href="README.md">한국어</a> · English</p>

Type everything on the QWERTY layout. The input method decides **per word** whether it is English, Korean
(2-set) or, when turned on, Japanese (romaji), and puts in the right characters. No Han/Eng key.

```
dkssudgktpdy hello              →  안녕하세요 hello
dlrjs githubdp commitgkf rjdi.  →  이건 github에 commit할 거야.
hellodnjfem githubtjqj          →  hello월드 github서버
fork flemal                     →  fork 리드미
```

A **fork** of the macOS input method [aimdevlee/seamtype](https://github.com/aimdevlee/seamtype): its
decision engine **ported to Rust and turned into a Windows input method (TSF)**. For macOS, see the original
repository.

## Install

Windows 10/11, 64-bit. Building needs [Rust](https://rustup.rs) and Visual Studio Build Tools (C++).

```powershell
powershell -ExecutionPolicy Bypass -File windows\scripts\install.ps1
```

- It builds, copies into `%ProgramFiles%\Seamtype` and registers the text service (one administrator prompt).
- It puts Seamtype first among your Korean keyboards and makes it the default input method, so Windows starts
  with it after a restart; add `-NotDefault` to leave that alone.
- For 32-bit apps (KakaoTalk and others), run `rustup target add i686-pc-windows-msvc` first; the x86 copy is
  then built and registered too.
- Updating is the same command. Apps already running use the new version once restarted.
- To remove it: `powershell -ExecutionPolicy Bypass -File windows\scripts\uninstall.ps1`
  (`-RemoveSettings` also drops settings and learned words).

## Keys

The word being typed is underlined and shown in the language the input method expects.

| Key | What it does |
| --- | --- |
| Letters | Go on with the word |
| **Esc** | Switch reading: English → English + particle → Korean → Japanese |
| **⇧Esc** | Switch reading, also in apps set to get Esc |
| Backspace | English: one letter. Korean: one jamo of the character being typed (한국 → 한구 → 한ㄱ) |
| Space, punctuation | Commit, then the key |
| Return | Commit only (press again for a new line or to send) |
| **Han/Eng key** | Pause / resume in this text field (for lazygit, htop and other places where keys are commands) |

- **Fixing and learning**: Esc right after a commit switches that word to its next reading. The same
  correction made twice for the same keys is remembered.
- **Menu**: right-click the Seamtype icon on the taskbar (**가A**, **A** while paused) for per-app settings
  (conversion, Standard/Strict, Esc to the app), Japanese input, undoing or forgetting learned words, and the
  settings folder.
- **Settings files**: `settings.json`, `apps.json` and `lexicon.json` in `%APPDATA%\Seamtype`. Apps are told
  apart by their executable name, lowercase.

```json
{ "*": { "escapeToApp": true }, "windowsterminal.exe": { "conversionThreshold": 0.97 } }
```

In terminals, `conversionThreshold` 0.97 (Strict) keeps short commands from turning into Hangul.

## What this fork adds

### A native Rust input method for Windows

- The macOS decision logic (`SeamtypeCore`, Swift) ported to Rust (`seamtype-core`). The port brought its
  133 Swift tests along, all passing, and matched the macOS engine on the word evaluation.
- A new TSF text service (`seamtype.dll`): composition (underline) and commit, the taskbar menu, pausing
  with the Han/Eng key, per-app settings, registration and install scripts.
- Dictionaries and models are baked into the DLL at build time: the input method runs inside every app,
  so no app has to read files again.

| Measured (release build) | Value |
| --- | --- |
| One key (Japanese off) | 7.8 µs average, 35 µs at the 99th percentile |
| An app's first key (models included) | under 1 ms |
| Memory per process | about 36 KB (65 KB with Japanese) |

### IMM32 apps (Webex, Tabby and others)

Apps that aren't TSF-aware go through Windows' IMM32 bridge, which applies Korean IME rules to a Korean
input method: only the last character is composed, and the ones before it go straight to the app. Fixed
on that path:

- Letters typed twice (`ddkkssss…`), the app ending compositions at once, lost keys, lost spaces.
- When a word changes language mid-way, characters already handed over are erased with Backspace and typed
  again. Showing an unsure English start as one Hangul character cut that from 6.4 to 2.3 times per 100
  words (a 20,000-word Korean chat simulation, same committed text).
- Tabby (xterm.js) notes where a composition ends only on a timer, and dropped characters handed over in
  quick succession (`fork` → `frk`, `claude` → `clade`). Characters now go in one at a time, giving the app
  time (30 ms) to catch up.
- The test harness emulates such apps (the one-character rule, the late timer, fast typing).

### Better decisions

- A loanword missing from the word lists (`리드미`) reads as Korean when it makes clean, common syllables,
  even right after an English word. On the top 10,000 words of each language in context, wrong words went
  from 488 to 456.

## Not yet

- **Kanji.** Japanese stops at kana: Space offers hiragana or katakana.
- Candidate window, settings window, vim integration, text replacements.
- In IMM32 apps, Esc right after a commit can't change the word (their text can't be read back).

## Development

```powershell
cd windows
cargo test                                         # core tests
cargo run -p seamtype-harness                      # end to end through Windows' TSF
cargo run -p seamtype-harness -- "dkssud hello "   # type a line and see the result
cargo run -p seamtype-cli -- -v "dkssud gksrmf hello"
python ..\tools\eval_context.py target\release\seamtype-cli.exe ko_50k.txt en_50k.txt 10000
```

| Path | Contents |
| --- | --- |
| `windows/crates/core` | Decision logic: composers, English/Korean/Japanese models, mixed words, learning, `InputSession` |
| `windows/crates/tip` | The TSF text service (`seamtype.dll`) |
| `windows/crates/cli` | `seamtype-cli`: decisions and their scores |
| `windows/crates/harness` | End-to-end checks through Windows' TSF, nothing registered |
| `windows/scripts` | Install and uninstall scripts |
| `Sources/SeamtypeCore/Data` | Dictionary and model data (shared with the original repository) |

For a diagnostic log, create an empty file named `debug` in `%APPDATA%\Seamtype`. `seamtype.log` then gets
key kinds and app names only, never the text typed.

## License

MIT ([LICENSE](LICENSE)). Follows the license and data notices of the original
[aimdevlee/seamtype](https://github.com/aimdevlee/seamtype); the notices for the data in the DLL are in
[`windows/ThirdPartyNotices.txt`](windows/ThirdPartyNotices.txt).
