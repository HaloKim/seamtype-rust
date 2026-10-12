# Seamtype for Windows

[English](README.en.md) · 한국어

영어(QWERTY) 자판 그대로 치면 단어마다 영어, 한국어(두벌식), 그리고 켜 두면 일본어(로마자)를 골라
주는 Seamtype을 Windows 입력기(TSF)로 옮긴 것입니다. 판정은 macOS판의 `SeamtypeCore`를 Rust로
그대로 옮긴 `seamtype-core`가 하므로, 같은 키에 같은 결과가 나옵니다. Swift 테스트 133개를 옮겨
모두 통과하고, `tools/eval_context.py`로 잰 단어 판정은 README의 표와 하나도 다르지 않습니다
(상위 1천/1만 단어 오답 48/488).

```
dkssudgktpdy hello      →  안녕하세요 hello
dlrjs githubdp commitgkf rjdi.  →  이건 github에 commit할 거야.
hellodnjfem githubtjqj  →  hello월드 github서버
```

## 설치

Windows 10/11, 64비트. 빌드에 [Rust](https://rustup.rs)와 Visual Studio Build Tools(C++)가 필요합니다.

```powershell
powershell -ExecutionPolicy Bypass -File windows\scripts\install.ps1
```

빌드한 뒤 `%ProgramFiles%\Seamtype`에 복사하고 등록합니다(관리자 확인 한 번). 그다음 한국어 키보드에
Seamtype을 추가하니, **Win+Space**로 한국어 → Seamtype을 고르면 됩니다. 목록에 없으면 설정 → 시간 및
언어 → 언어 및 지역 → 한국어 → 언어 옵션 → 키보드에서 추가하세요.

- 32비트 앱(카카오톡 등)에도 쓰려면 먼저 `rustup target add i686-pc-windows-msvc`를 실행하세요.
  설치 스크립트가 x86판도 빌드해서 함께 등록합니다.
- 업데이트도 같은 명령입니다. 이미 실행 중인 앱은 다시 시작해야 새 버전을 씁니다.
- 지우기: `powershell -ExecutionPolicy Bypass -File windows\scripts\uninstall.ps1`.
  설정과 학습한 단어까지 지우려면 `-RemoveSettings`를 붙이세요.

## 키

macOS판과 같습니다([README](../README.md#키)). 다른 점은 다음과 같습니다.

| macOS | Windows |
| --- | --- |
| 오른쪽 ⌘를 한 번 눌러 일시정지 | **한/영 키**(또는 작업 표시줄의 Seamtype 아이콘 왼쪽 클릭) |
| 메뉴 막대의 입력 메뉴 | 작업 표시줄의 Seamtype 아이콘(**가A**, 일시정지 중엔 **A**) 오른쪽 클릭 |
| 앱 이름: 번들 ID(`com.apple.Terminal`) | 실행 파일 이름, 소문자(`windowsterminal.exe`, `code.exe`, `kakaotalk.exe`) |
| `~/Library/Application Support/Seamtype` | `%APPDATA%\Seamtype` |

Esc(언어 바꾸기), ⇧Esc, 커밋 직후 Esc로 바로 고치기와 두 번 고치면 학습, Delete가 한글은 자모
단위·앞 음절은 통째로 지우는 것, 문장부호, Return이 확정만 하는 것, 앱별 설정(`apps.json`의 `"*"`,
보통/엄격, Esc를 앱으로)은 모두 그대로입니다.

메뉴에는 이 입력창에서 일시정지, 지금 앱의 설정(단어 변환, 변환 기준 보통/엄격, Esc 키를 앱으로, 기본
설정 따르기), 일본어 입력, 마지막 학습 취소, 기억한 단어 모두 잊기, 설정 폴더 열기가 있습니다.

## 아직 없는 것

- **한자 변환.** macOS판은 AzooKey의 한자 사전(Swift)을 씁니다. Windows판의 일본어는 가나까지라서,
  Space를 누르면 히라가나와 가타카나 중에 고릅니다(F7/F6, ⌃K/⌃J도 같음). 라이브 변환도 아직 없습니다.
- 후보 창, 설정 창(입력해 보기, 기억한 단어 목록), 일시정지할 때 커서 아래에 뜨는 표시, vim 연동,
  텍스트 대치는 아직 없습니다. 설정은 메뉴나 JSON 파일로 바꿉니다.
- 잠금 화면에서는 쓰지 않습니다.

## 설정 파일

`%APPDATA%\Seamtype`의 `settings.json`, `apps.json`, `lexicon.json`은 macOS판과 형식이 같습니다
([README](../README.md#설정-파일)). 손으로 고치면 다음에 입력창을 누를 때(입력 중이면 0.5초 안에)
반영됩니다. `rightCommandPause`는 Windows에선 한/영 키 일시정지를 켜고 끕니다.

```json
{ "*": { "escapeToApp": true }, "windowsterminal.exe": { "conversionThreshold": 0.97 } }
```

터미널(Windows Terminal에서 쓰는 WSL, PowerShell)에서는 `conversionThreshold`를 0.97(엄격)로 두면
짧은 명령이 한글로 바뀌지 않습니다. 메뉴에서 "변환 기준: 엄격하게"를 고르면 됩니다.

## 구조

| 경로 | 내용 |
| --- | --- |
| `crates/core` | `SeamtypeCore`의 Rust 이식: 조합기, 영어·한국어·일본어 모델, 혼합 단어, 학습, `InputSession`, 설정 |
| `crates/tip` | TSF 텍스트 서비스(`seamtype.dll`): 키 → `InputSession`, 조합(밑줄)과 커밋, 작업 표시줄 메뉴, 등록 |
| `crates/cli` | `seamtype-cli`: macOS판 CLI와 같은 명령(`-v`, `--decide`, `--compose`, `--kana`) |
| `crates/harness` | 입력기를 등록하지 않고 Windows의 TSF로 끝까지 입력해 보는 검사 |
| `scripts` | 설치·제거 스크립트 |

데이터는 macOS판과 같은 `Sources/SeamtypeCore/Data`를 `build.rs`가 빌드할 때 표로 바꿔 DLL에 넣습니다.
입력기는 앱마다 그 앱 안에서 돌기 때문에, 표를 DLL에서 바로 읽어 프로세스마다 다시 읽거나 메모리를
더 쓰지 않습니다. 영어 철자 모델이 쓰는 macOS의 `/usr/share/dict/words`(web2)는 Windows에 없으므로,
Apple 배포본에서 센 글자 3-gram을 `crates/core/data/spelling_trigrams.txt`로 넣었습니다
(`tools/build_spelling_trigrams.py`). 판정 한 번은 1ms 안쪽입니다(키 입력당 평균 0.03ms).

## 개발

```powershell
cargo test                                   # 코어(Swift에서 옮긴 테스트)와 키 매핑
cargo run -p seamtype-harness                # TSF로 끝까지: 조합, 커밋, Esc 고치기, 한/영, 포커스 이동
cargo run -p seamtype-harness -- "dkssud hello "   # 한 줄 입력해 보고 결과 보기
cargo run -p seamtype-cli -- -v "dkssud gksrmf hello"
python tools\eval_context.py target\release\seamtype-cli.exe ko_50k.txt en_50k.txt 10000
```

진단 로그가 필요하면 `%APPDATA%\Seamtype`에 `debug`라는 빈 파일을 만드세요. 그 폴더의
`seamtype.log`에 키 종류와 앱만 남고, 입력한 글자는 남지 않습니다.
