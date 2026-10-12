# Seamtype for Windows

English · [한국어](README.md)

Seamtype as a Windows input method (Text Services Framework): type everything on QWERTY and each
word comes out as English, Korean (2-set) or, if you turn it on, Japanese (romaji). The decisions
are made by `seamtype-core`, a Rust port of the macOS `SeamtypeCore`, so the same keys give the same
words. The 133 ported Swift tests pass, and `tools/eval_context.py` gives the README's table exactly
(48 / 488 wrong among the top 1,000 / 10,000 words).

```
dkssudgktpdy hello      →  안녕하세요 hello
dlrjs githubdp commitgkf rjdi.  →  이건 github에 commit할 거야.
hellodnjfem githubtjqj  →  hello월드 github서버
```

## Install

Windows 10/11, 64-bit. Building needs [Rust](https://rustup.rs) and the Visual Studio Build Tools (C++).

```powershell
powershell -ExecutionPolicy Bypass -File windows\scripts\install.ps1
```

It builds, copies into `%ProgramFiles%\Seamtype` and registers the text service (one administrator
prompt), then adds Seamtype to your Korean keyboards: press **Win+Space** and choose Korean → Seamtype.
If it isn't listed, add it in Settings → Time & language → Language & region → Korean → Language
options → Keyboards.

- For 32-bit apps (KakaoTalk and others) run `rustup target add i686-pc-windows-msvc` first; the script
  then builds and registers the x86 copy too.
- Updating is the same command. Apps already running use the new version once restarted.
- To remove it: `powershell -ExecutionPolicy Bypass -File windows\scripts\uninstall.ps1`
  (`-RemoveSettings` also drops settings and learned words).

## Keys

As on macOS ([README](../README.en.md#keys)), except:

| macOS | Windows |
| --- | --- |
| Tap right ⌘ to pause | The **Han/Eng (한/영) key**, or a left click on Seamtype's taskbar icon |
| The input menu in the menu bar | A right click on Seamtype's taskbar icon (**가A**, **A** while paused) |
| Apps by bundle identifier (`com.apple.Terminal`) | By program file, lowercase (`windowsterminal.exe`, `code.exe`) |
| `~/Library/Application Support/Seamtype` | `%APPDATA%\Seamtype` |

Esc and ⇧Esc to switch readings, Esc right after a commit (and learning a correction made twice),
Delete by jamo in the syllable being typed and whole syllables before it, punctuation, Return that only
confirms, and per-app behavior (`apps.json` with `"*"`, Normal/Strict, Esc to the app) all work the same.

The menu has: pause in this text field, the app's settings (convert words, Normal/Strict, Esc key to the
app, use the default), Japanese input, undo the last learned word, forget all remembered words, and open
the settings folder.

## IMM32 apps (Webex and others)

Apps that aren't TSF-aware (Qt apps such as Webex, older Win32 apps) go through Windows' IMM32
bridge, which applies Korean IME rules to a Korean input method: only the last character is
composed. There Seamtype composes like the system's Korean input, the last character underlined and
the ones before it typed. Decisions are still per word: when a word changes reading mid-way (Esc, a
split decided late), the characters already typed are erased with Backspace and typed again. Esc
right after a commit can't change the word in these apps (their text can't be read back).

## Not yet

- **Kanji.** macOS converts with AzooKey's dictionary (Swift). On Windows, Japanese stops at kana: Space
  offers the hiragana and katakana forms (as do F7/F6 and ⌃K/⌃J). No live conversion yet.
- No candidate window, settings window (Try Typing, the list of remembered words), pause indicator under
  the caret, vim integration or text replacements yet; settings are changed from the menu or the JSON files.
- Not used on the lock screen.

## Settings files

`settings.json`, `apps.json` and `lexicon.json` in `%APPDATA%\Seamtype` have the macOS format
([README](../README.en.md#settings-files)). Edits by hand apply the next time a text field gets focus
(within half a second while typing). On Windows `rightCommandPause` turns the Han/Eng pause key on or off.

```json
{ "*": { "escapeToApp": true }, "windowsterminal.exe": { "conversionThreshold": 0.97 } }
```

In a terminal (WSL or PowerShell in Windows Terminal) a threshold of 0.97 (Strict) keeps short commands
from turning into Hangul; choose "Conversion: Strict" in the menu.

## Layout

| Path | What |
| --- | --- |
| `crates/core` | The Rust port of `SeamtypeCore`: composers, the English, Korean and Japanese models, mixed words, learning, `InputSession`, settings |
| `crates/tip` | The TSF text service (`seamtype.dll`): keys → `InputSession`, composition (underlined) and commits, the taskbar menu, registration |
| `crates/cli` | `seamtype-cli`, with the macOS CLI's commands (`-v`, `--decide`, `--compose`, `--kana`) |
| `crates/harness` | Types through Windows' TSF end to end, without registering the input method |
| `scripts` | Install and uninstall |

The data is the macOS app's `Sources/SeamtypeCore/Data`, turned into tables by `build.rs` and compiled
into the DLL. An input method runs inside every app, so the tables are read straight from the DLL: no
process parses them or keeps its own copy. The English spelling model trains on macOS's
`/usr/share/dict/words` (web2), which Windows lacks, so its letter trigrams, counted from Apple's copy,
ship as `crates/core/data/spelling_trigrams.txt` (`tools/build_spelling_trigrams.py`). A decision takes
well under a millisecond (0.03 ms per keystroke on average).

## Develop

```powershell
cargo test                                   # the core (ported Swift tests) and the key mapping
cargo run -p seamtype-harness                # through TSF: composition, commits, Esc, Han/Eng, focus changes
cargo run -p seamtype-harness -- "dkssud hello "   # type one line and show what it left
cargo run -p seamtype-cli -- -v "dkssud gksrmf hello"
python tools\eval_context.py target\release\seamtype-cli.exe ko_50k.txt en_50k.txt 10000
```

For a diagnostic log, create an empty file named `debug` in `%APPDATA%\Seamtype`; `seamtype.log` there
gets the kind of key and the app only, never typed text.
