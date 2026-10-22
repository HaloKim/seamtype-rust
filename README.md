<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/media/logo-dark.svg">
    <img src="docs/media/logo-light.svg" width="150" alt="Seamtype: A, 아, あ">
  </picture>
</p>

<h1 align="center">seamtype-rust</h1>

<p align="center"><b>한/영 키 없이 치는 Windows 입력기</b></p>

<p align="center">한국어 · <a href="README.en.md">English</a></p>

모든 글을 영문(QWERTY) 자판 그대로 치세요. 입력기가 **단어마다** 영어인지 한국어(두벌식)인지, 켜 두었다면
일본어(로마자)인지 판단해서 맞는 글자로 넣습니다. 한/영 키를 누를 일이 없습니다.

```
dkssudgktpdy hello              →  안녕하세요 hello
dlrjs githubdp commitgkf rjdi.  →  이건 github에 commit할 거야.
hellodnjfem githubtjqj          →  hello월드 github서버
fork flemal                     →  fork 리드미
```

macOS 입력기 [aimdevlee/seamtype](https://github.com/aimdevlee/seamtype)의 판정 엔진을 **Rust로 옮겨
Windows 입력기(TSF)로 만든 포크**입니다. macOS판은 원본 저장소를 보세요.

## 설치

Windows 10/11, 64비트.

1. [Releases](https://github.com/HaloKim/seamtype-rust/releases/latest)에서 `Seamtype-Setup-<버전>.exe`를 받습니다.
2. 실행해서 안내대로 설치합니다(관리자 확인 한 번). "Seamtype을 기본 입력기로 지정"을 켜 두면 재부팅해도
   Seamtype으로 시작합니다.
3. **Win+Space**로 한국어 → Seamtype을 고릅니다.

업데이트는 새 설치 파일을 다시 실행하면 되고, 제거는 설정 → 앱 → 설치된 앱의 Seamtype에서 합니다. 이미
실행 중인 앱은 다시 시작해야 새 버전을 씁니다.

### 소스에서 빌드해 설치

[Rust](https://rustup.rs)와 Visual Studio Build Tools(C++)가 필요합니다.

```powershell
powershell -ExecutionPolicy Bypass -File windows\scripts\install.ps1
```

- 빌드한 뒤 `%ProgramFiles%\Seamtype`에 복사하고 등록합니다(관리자 확인 한 번).
- 한국어 키보드 목록 맨 앞에 Seamtype을 넣고 기본 입력 방법으로 지정하므로, 재부팅해도 Seamtype으로
  시작합니다. 원하지 않으면 `-NotDefault`를 붙이세요.
- 32비트 앱(카카오톡 등)에도 쓰려면 먼저 `rustup target add i686-pc-windows-msvc`를 실행하세요.
  x86판도 함께 빌드해 등록합니다.
- 업데이트도 같은 명령입니다. 이미 실행 중인 앱은 다시 시작해야 새 버전을 씁니다.
- 제거: `powershell -ExecutionPolicy Bypass -File windows\scripts\uninstall.ps1`
  (설정과 학습한 단어까지 지우려면 `-RemoveSettings`).

## 키

치고 있는 단어에는 밑줄이 그어지고, 입력기가 예상한 언어로 보입니다. Chrome, Tabby처럼 IMM32를 거치는 앱에서는
단어가 끝날 때까지 커서 아래 작은 창에 보이고, Space·문장부호·Return에서 한 번에 들어갑니다.

| 키 | 동작 |
| --- | --- |
| 글자 | 단어를 이어서 침 |
| **Esc** | 읽기 바꾸기: 영어 → 영어+조사 → 한국어 → 일본어 |
| **⇧Esc** | 읽기 바꾸기 (Esc를 앱에 넘기도록 한 앱에서도) |
| Backspace | 영어는 한 글자, 한국어는 치고 있는 글자의 자모 하나씩 (한국 → 한구 → 한ㄱ) |
| Space, 문장부호 | 확정하고 그 키를 넣음 |
| Return | 확정만 (줄바꿈·전송은 한 번 더) |
| **한/영 키** | 이 입력창에서 일시정지 / 다시 켜기 (lazygit, htop처럼 키가 명령인 곳에서) |

- **고치기와 학습**: 확정한 직후 Esc를 누르면 그 단어가 다음 읽기로 바뀝니다. 같은 단어를 같은 쪽으로
  두 번 고치면 그때부터 기억합니다.
- **메뉴**: 작업 표시줄의 Seamtype 아이콘(**가A**, 일시정지 중엔 **A**)을 오른쪽 클릭하면 앱별 설정
  (단어 변환, 변환 기준 보통/엄격, Esc를 앱으로), 일본어 입력, 학습 취소·초기화, 설정 폴더 열기가 있습니다.
- **설정 파일**: `%APPDATA%\Seamtype`의 `settings.json`, `apps.json`, `lexicon.json`. 앱은 실행 파일
  이름(소문자)으로 구분합니다.

```json
{ "*": { "escapeToApp": true }, "windowsterminal.exe": { "conversionThreshold": 0.97 } }
```

터미널에서는 `conversionThreshold`를 0.97(엄격)로 두면 짧은 명령이 한글로 바뀌지 않습니다.

## 이 포크에서 한 작업

### Rust 네이티브 Windows 입력기

- macOS판의 Swift 판정 로직(`SeamtypeCore`)을 Rust(`seamtype-core`)로 옮겼습니다. 옮길 때 Swift 테스트
  133개를 함께 옮겨 모두 통과했고, 단어 판정 평가 결과도 macOS판과 같았습니다.
- TSF 텍스트 서비스(`seamtype.dll`)를 새로 만들었습니다: 조합(밑줄)과 확정, 작업 표시줄 메뉴, 한/영 키
  일시정지, 앱별 설정, 등록·설치 스크립트.
- 사전과 모델은 빌드할 때 표로 바꿔 DLL에 넣습니다. 입력기는 모든 앱 안에서 돌기 때문에, 앱마다 파일을
  다시 읽지 않도록 한 것입니다.

| 측정 (Release 빌드) | 값 |
| --- | --- |
| 키 하나 처리 (일본어 끔) | 평균 7.8 µs, 상위 1% 35 µs |
| 앱의 첫 키 (모델 준비 포함) | 1 ms 안쪽 |
| 프로세스당 메모리 | 약 36 KB (일본어 켜면 65 KB) |

### IMM32 앱 지원 (Webex, Tabby 등)

TSF를 직접 쓰지 않는 앱은 Windows의 IMM32 호환 계층을 거칩니다. 여기서는 한국어 입력기에 "조합 중인
글자는 마지막 한 글자뿐" 규칙이 적용돼, 앞 글자는 바로 앱으로 넘어갑니다. 그래서 단어 중간에 언어가 바뀌면
이미 넘긴 글자를 Backspace로 지우고 다시 써야 했고, 앱마다 이걸 다르게 받아 글자를 잃었습니다(Tabby,
Chrome 주소창, 웹 Excel).

- **기본: 단어를 작은 창에 보류.** 치는 동안에는 앱에 아무것도 넘기지 않고 커서 아래 Seamtype 창에
  보여 주다가, 단어가 끝날 때 한 번에 넣습니다. 지울 일이 없으니 앱별 문제가 생기지 않습니다.
- 메뉴의 "치는 단어를 작은 창에 보이기"를 끄면(`settings.json`의 `"imeImmInline": true`) 예전처럼 앱 안에서
  조합합니다. 이 방식에서 고친 것들:

  - 글자가 두 번씩 들어가던 문제(`ddkkssss…`), 앱이 조합을 바로 끝내 버리던 문제, 키 유실, 띄어쓰기 누락.
  - 영어인지 확실하지 않은 앞부분은 한글 한 글자로 보여 줘서, 지우고 다시 쓰는 보정을 100단어당 6.4번에서
    2.3번으로 줄였습니다(한국어 채팅 2만 단어 시뮬레이션, 확정 결과는 동일).
  - Tabby(xterm.js)는 조합 위치를 타이머로 늦게 기록해서, 글자를 빠르게 이어 넘기면 버렸습니다
    (`fork` → `frk`, `claude` → `clade`). 한 번에 한 글자씩, 앱이 따라올 시간(30ms)을 두고 넘깁니다.
  - 웹 Excel은 Backspace마다 포커스를 잠깐 옮기고, 비워진 셀을 초기화합니다(`개인적인` → `적인`).
- 테스트 하네스가 이런 앱의 동작(한 글자 조합 규칙, 늦게 기록하는 타이머, 빠른 타자, 포커스 이동, 셀
  초기화)을 흉내 내서 두 방식 모두 검사합니다.

### 판정 개선

- 단어 목록에 없는 외래어(`리드미`)도 남는 자모 없이 흔한 음절로 읽히면 한국어로 판단합니다. 영어 단어
  바로 뒤에서도 마찬가지입니다. 흔한 단어 1만 개씩의 문맥 평가에서 오답이 488개에서 456개로 줄었습니다.

## 아직 없는 것

- **한자 변환.** 일본어는 가나까지라서 Space로 히라가나와 가타카나 중에 고릅니다.
- 후보 창, 설정 창, vim 연동, 텍스트 대치.
- IMM32 앱에서는 확정한 뒤 Esc로 고치기가 되지 않습니다(앱의 글자를 읽을 수 없음).

## 개발

```powershell
cd windows
cargo test                                         # 코어 테스트
cargo run -p seamtype-harness                      # Windows TSF로 끝까지 입력해 보는 검사
cargo run -p seamtype-harness -- "dkssud hello "   # 한 줄 입력해 보고 결과 보기
cargo run -p seamtype-cli -- -v "dkssud gksrmf hello"
python ..\tools\eval_context.py target\release\seamtype-cli.exe ko_50k.txt en_50k.txt 10000
```

| 경로 | 내용 |
| --- | --- |
| `windows/crates/core` | 판정 로직: 조합기, 영어·한국어·일본어 모델, 혼합 단어, 학습, `InputSession` |
| `windows/crates/tip` | TSF 텍스트 서비스(`seamtype.dll`) |
| `windows/crates/cli` | `seamtype-cli`: 판정 결과와 점수 확인 |
| `windows/crates/harness` | 입력기를 등록하지 않고 Windows TSF로 끝까지 입력해 보는 검사 |
| `windows/scripts` | 설치·제거 스크립트 |
| `windows/installer` | 설치 파일(Inno Setup): `build.ps1`이 `windows\target\installer`에 만듦. `v*` 태그를 올리면 GitHub Actions가 만들어 Releases에 올림 |
| `Sources/SeamtypeCore/Data` | 사전과 모델 데이터 (원본 저장소와 공유) |

진단 로그가 필요하면 `%APPDATA%\Seamtype`에 `debug`라는 빈 파일을 만드세요. `seamtype.log`에 키 종류와
앱 이름만 남고, 입력한 글자는 남지 않습니다.

## 라이선스

MIT ([LICENSE](LICENSE)). 원본 [aimdevlee/seamtype](https://github.com/aimdevlee/seamtype)의 라이선스와
데이터 고지를 따르며, DLL에 들어가는 데이터의 고지는
[`windows/ThirdPartyNotices.txt`](windows/ThirdPartyNotices.txt)에 있습니다.
