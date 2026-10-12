# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Seamtype is a macOS InputMethodKit input method: the user types on QWERTY and Seamtype decides per
word whether it is English, Korean (2-set) or Japanese romaji (kanji through AzooKey). README.md
(Korean, the main one) and README.en.md (English) cover the keys, settings files, accuracy numbers
and data licenses; read one before changing behavior users see, and keep both in step. The website,
https://nyttastudio.com/seamtype/ (Korean only), lives in `seamtype/` of the nyttastudio/nyttastudio.com
repo (Cloudflare Pages); keep its copy in step too.

## Commands

```sh
swift build                                  # debug build of everything
swift test                                   # all tests (Swift Testing, ~7 s)
swift test --filter MixedWordTests           # one suite; --filter 'MixedWordTests/splitsEnglishPlusParticle' for one test
scripts/install.sh                           # release build + --selftest, install to ~/Library/Input Methods, enable, restart menu agent
scripts/release.sh                           # universal build → build/release/Seamtype{,-<version>}.{zip,pkg}
                                             # (SEAMTYPE_APP_SIGN / SEAMTYPE_PKG_SIGN / SEAMTYPE_NOTARY_PROFILE: Developer ID, notarization)
swift run seamtype-cli -v "dkssud hello"     # English/Korean decision with scores
printf 'keys\tprevious\n' | swift run seamtype-cli --decide --japanese   # previous: english|korean|japanese|empty
printf 'きょうは\n' | swift run seamtype-cli --convert                    # AzooKey candidates
tools/prepare.sh [--models]                  # held-out accuracy (downloads into tools/data, gitignored)
scripts/typing-test/run.sh                   # real key events into TextEdit through the installed app
/usr/bin/log show --last 10m --predicate 'subsystem == "com.aimdevlee.inputmethod.Seamtype"' --info --debug
```

- `scripts/typing-test/run.sh` takes over the keyboard and focus for about a minute. Don't run it
  while the user is using the machine.
- Use `/usr/bin/log`, not `log`: zsh's builtin `log` shadows it and prints nothing.
- A change only reaches real typing after `scripts/install.sh`. A copy started with `open` or from
  `build/` never gets IMK connections; macOS has to launch the installed bundle itself.

## Architecture

**Targets.** `SeamtypeCore` holds all the typing logic and is what the tests cover. `Seamtype`
is a thin IMK layer over it. `seamtype-cli` is for tuning and evaluation.

**Decision chain** (built in `Runtime.makeEngine`): `LearningEngine` (lexicon.json: a correction
made twice wins) → `SplittingEngine` (`MixedWord.swift`) → `HeuristicEngine`.
- `SplittingEngine` handles English + Korean particle (`github에`), a single letter + particle
  (`xdp` → `x에`, not 도/나 after a lowercase letter), English + whole Korean word (`hello월드`,
  using `Data/korean_vocabulary.txt`, which is only for this) and English + Japanese particle
  (`githubで`). Its decisions have confidence 0.98 (`SplittingEngine.confidence`), above Strict
  (0.97), so they split in a strict app too.
- `HeuristicEngine` scores the keys per keystroke as English, Korean and Japanese, then nudges
  toward the previous word's language.
- `render(_:as: .mixed)` uses the engine's split when the engine makes one, else the plain
  particle split (what an Esc-forced mixed reading shows).

**`InputSession`** is one state machine per text client, behind the `TextClient` protocol (the
tests use `FakeTextField`). It owns the preview and hysteresis, commits, Esc reading cycling, the
"flip the word just committed" window, kana-kanji conversion and live conversion.
- Some splits happen here, not in the engine, because the part after the split keeps composing
  (the Japanese half needs conversion): Korean → English or Japanese (`오늘은` + `google에서`,
  `이력` + `rireki`) and Japanese → Korean (`rireki` + `를`, `kyouha` + `도시락`). The front part
  is committed mid-word (Japanese as kana, or the live candidate) and the rest keeps composing.
- Korean → other fires only when the whole token reads as broken Hangul, and for Japanese also
  not as Japanese on its own; Japanese → Korean only when the romaji has stray letters. Without
  those guards, ordinary Japanese phrases split (`chanwokoroshita` → `초무をころした`).
- In a Strict app, a token of 3 keys or fewer kept English by the threshold wins over a Korean
  preview shown along the way (`gst` stays `gst`); longer words keep the preview's hysteresis.
- Return on an underlined word only confirms it (`returnConfirmsOnly`, on by default); off, the
  app gets the Return too, except after Japanese.
- Vim reports its modes through `scripts/vim/seamtype.{lua,vim}` (`Seamtype --vim <mode> --app
  $__CFBundleIdentifier`, a distributed notification). `Runtime` hands it to the active controller,
  which takes it only if vim runs in its app (a signal without `--app`, only where Esc goes to the
  app). Outside insert mode every key passes straight through, and while
  vim reports, Esc is vim's whatever the app's policy says; nothing guesses the mode from keys.
- A right ⌘ tap (flagsChanged, in `InputController`) pauses the session until the next tap,
  `activate`, or a settings change (a fresh session). The menu icon can't change, so `PauseHUD`
  shows the icon under the caret for a moment, struck through when paused (drawn by
  `make-icon.swift --slash`), for the menu's toggle too. It copies the system's input source
  indicator, measured on screen: size, gap, and the accent color's components taken as Display P3.
- After a word ends with Space or punctuation, only the word is inserted and `handle` returns
  false, so the app types the key itself, as the system Korean IME does. Inserting "word + space"
  left a stray character in Ghostty. Test harnesses must insert the passed-on key themselves.

**`Runtime`** (`Sources/Seamtype`) is a process-wide singleton holding settings, engine and
converter, plus a `generation` counter. Controllers swap in a fresh session when the generation
changes, so a settings change applies without a restart.

**Settings window.** It is a separate process, `Seamtype --settings <tab>`, because the IME is
`LSBackgroundOnly`. It edits the JSON files in `~/Library/Application Support/Seamtype/` and posts
the distributed notification `com.aimdevlee.inputmethod.Seamtype.settingsChanged`; the IME reloads
on it. About's Uninstall… (`Uninstaller.swift`, shown only in a copy under an Input Methods folder)
deletes both installed copies and the pkg receipt, so testing it uninstalls the real input method:
run `scripts/install.sh` again afterwards.

**Per-app policy** (`AppPolicy.swift`): in `apps.json`, `"*"` is the user's default and an app
entry names only the keys that differ; a key ending in `.` matches as a prefix. There is no
built-in app list, so add an entry only when the user asks for one.

**Resources.** Data files are found by `Resources.swift`: `Contents/Resources` in the app, the
source tree in development. SPM resource bundles are avoided on purpose, because they'd sit outside
the signed `Contents`. `build-app.sh` copies each data file explicitly, so a new file under
`Sources/SeamtypeCore/Data` has to be added there too.

**Menu icon.** The system reads it from `Seamtype.tiff` (drawn by `scripts/make-icon.swift`),
and it's fixed: the bundle is signed (Developer ID for releases), so nothing inside it may change at
runtime.

## Conventions and pitfalls

- **Logging:** never log typed text. The input logger records the key kind and app only.
- **UI strings:** they go through `L("English key")`, with Korean in
  `Resources/ko.lproj/Localizable.strings`. The input source's name comes from
  `Resources/*.lproj/InfoPlist.strings`.
- **New split or heuristic rules:** check them for wrong splits, not just the target case.
  - Run every keys row of `tools/data/eval.tsv` (English, Korean, Japanese) through an
    `InputSession` with Japanese on, and count Hangul next to kana.
  - For English, also run `tools/data/en_50k.txt` through `seamtype-cli -v`.
  - The CLI only shows the engine's decision, so also test through a session: an earlier split
    looked right in the CLI but never reached real typing.
- **Install refreshes:** after a reinstall, the input menu keeps its old list until
  `TextInputMenuAgent` restarts, and the switch indicator by the caret keeps the icon it read
  first until `CursorUIViewService` and `TextInputSwitcher` do (they ignore SIGTERM, so the install
  paths SIGKILL them). A fresh process can still take 10–30 s to see the source as enabled, hence
  `enable-input-source.swift --check` with retries.
- **Releases** are signed with the Developer ID and notarized, the app and the pkg alike; run
  `release.sh` with `SEAMTYPE_APP_SIGN="Developer ID Application: Dongbin Lee (2UZ65T3U82)"`,
  `SEAMTYPE_PKG_SIGN="Developer ID Installer: Dongbin Lee (2UZ65T3U82)"` and
  `SEAMTYPE_NOTARY_PROFILE=seamtype`, or the assets come out ad hoc and unsigned. Upload
  `Seamtype.pkg` (the website's download button) and `Seamtype.zip` (`scripts/get.sh`) under those
  names for `releases/latest/download/`, plus the versioned copies.
  `SEAMTYPE_ZIP=build/release/Seamtype.zip sh scripts/get.sh` tries the zip locally. The pkg installs
  into `/Library/Input Methods` and is not relocatable; its postinstall runs `Seamtype --register` as
  the logged-in user (users have no Swift toolchain, so registration can't use the scripts).
  `release.sh` signs the app with the hardened runtime even ad hoc, and `build-app.sh` runs the
  self-test under Rosetta too for a universal build.
- **Stale registrations:** extra copies of the app registered with LaunchServices (`build/`,
  the old `AutoIME.app`) can make macOS launch the wrong one. `build-app.sh` unregisters its copy.

## Windows (`windows/`)

A Rust workspace: `crates/core` (seamtype-core) ports `Sources/SeamtypeCore` module for module and
reads the same `Sources/SeamtypeCore/Data` files (baked into tables by `build.rs`); `crates/tip` is
the TSF text service (`seamtype.dll`, the counterpart of `Sources/Seamtype`); `crates/cli` mirrors
seamtype-cli; `crates/harness` types through Windows' real TSF into a text store of its own.

```sh
cd windows && cargo test                     # ported Swift tests + key mapping
cargo run -p seamtype-harness                # end to end through TSF, nothing registered
python3 ../tools/eval_context.py target/release/seamtype-cli.exe ko_50k.txt en_50k.txt 10000   # 48 / 488
powershell -ExecutionPolicy Bypass -File scripts/install.ps1   # build, copy to Program Files, regsvr32 (UAC), add the keyboard
```

- **Keep the port in step.** A behavior change in `SeamtypeCore` goes into `crates/core` the same
  way, with its test ported; `eval_context.py` must still match between the two CLIs.
- The text service runs inside every app: never panic across COM (`guard`), never hold the runtime
  lock or a `RefCell` borrow while calling into TSF (it calls back), and never log typed text.
- TSF routes keys and focus events only to a text service it activated from the registry, so the
  harness calls the key and focus sinks itself; edit sessions and compositions are TSF's own.
- Settings changes reach other processes through the files' modification times (no distributed
  notifications on Windows); `LearningEngine` reads lexicon.json back before writing.
- Apps that aren't TSF-aware (Webex and other Qt apps, Win32 edit controls) reach TSF through
  Windows' IMM32 bridge (CUAS), whose documents are transitory. Under a Korean profile it follows
  Korean IME rules: the keyboard must be open in native mode on the thread manager's compartments
  (not the global ones), only the last character composes (an interim character) and everything
  before it goes to the app as typed. `ImmWord` in client.rs composes such words one character at
  a time and never sends delivered text again; changing it means injected Backspaces (with
  `inject::MAGIC`) and a marker key (F24) that resumes the word. The harness covers it with a
  transitory store and an injector hook.
- Some apps hand each key press to the text service twice (Webex, a Qt app reaching TSF through
  Windows' IMM32 bridge, typed `ddkkssss…`): a key-down with the same key and lParam as the last
  one, before its key-up and not auto-repeat, is the same press (`DUPLICATE_WINDOW`).
- Windows has no `/usr/share/dict/words`: `crates/core/data/spelling_trigrams.txt` holds its counts
  (`windows/tools/build_spelling_trigrams.py`).
