# Local Realm Rebuilt

Local Realm 1.9.0의 배포 폴더를 바탕으로 다시 작성한 Rust/Tauri 데스크톱 앱입니다. 원본 실행 파일을 수정하거나 실행하는 방식이 아닙니다. 새 Rust 백엔드와 복구한 웹 화면을 함께 빌드합니다.

현재 버전은 **1.9.0-rebuilt.1 — 재구현 검증판**입니다. 복구된 화면이 사용하는 183개 명령을 구현했지만, 이것만으로 원본과 모든 동작이 같다고 보장하지 않습니다. 정확한 검증 범위와 차이는 [호환성 문서](docs/COMPATIBILITY.md)에 기록합니다.

## 실행

Windows x64 릴리즈 ZIP을 풀고 `local-realm-rebuilt.exe`를 실행합니다. ZIP에 포함된 `brushes`, `WebView2Loader.dll`, WebView2 런타임 폴더는 실행 파일 옆에 두세요. 고정 런타임이 없는 빌드는 설치된 Microsoft Edge WebView2를 사용합니다.

기존 세계관은 복사본을 열어 확인할 수 있습니다. 앱에서 만든 스냅샷은 세계관 폴더의 `backups`에 저장되며, 빈 폴더로 복원합니다.

## 빌드 및 검사

필요 도구: Windows, Node.js 22 이상, Rust stable, Visual Studio C++ Build Tools와 Windows SDK. Cargo.lock에 Rust 의존성을 고정했습니다. Node 패키지 설치는 필요하지 않습니다.

```powershell
npm test
./tools/build.ps1 -Action test
./tools/build.ps1 -Action check
./tools/build.ps1 -Action release
```

빌드 결과는 `dist/rebuilt/`에 생성됩니다. `check`는 Clippy 경고도 실패로 처리합니다. GNU Rust 도구가 `%LOCALAPPDATA%/LocalRealmBuild`에 설치된 환경은 빌드 스크립트가 자동으로 사용하며, 한글 경로를 처리하지 못하는 GNU 리소스 컴파일러를 위해 ASCII 경로에 빌드용 복사본을 만듭니다. 다른 환경에서는 일반 Cargo/MSVC를 사용합니다.

```powershell
./tools/collect-licenses.ps1
./tools/package.ps1 -SkipBuild
# 고정 WebView2 런타임을 함께 넣으려면:
./tools/package.ps1 -SkipBuild -RuntimePath 'C:/path/to/Microsoft.WebView2.FixedVersionRuntime...'
```

패키징은 `deliverables/`에 소스 ZIP, Windows ZIP, SHA-256 체크섬을 생성합니다. GitHub 업로드나 릴리즈 게시를 자동으로 수행하지 않습니다.

## 구조

| 경로 | 내용 |
| --- | --- |
| `src-tauri/src/` | 새로 작성한 Rust 도메인별 구현 |
| `recovered-backend/migrations/` | 원본에 포함돼 있던 SQLite 스키마 변경 이력 |
| `recovered-frontend/` | 실행 파일에서 복구한 배포용 HTML·JS·CSS·이미지 |
| `frontend/` | 직접 수정할 수 있는 화면 성능 개선 및 호환 어댑터 |
| `tools/build-frontend.mjs` | 복구한 화면에 검증된 변경을 적용하는 빌드 과정 |
| `assets/brushes/` | 지도 브러시와 출처 정보 |
| `src-tauri/resources/` | 기본 분류, 관계 종류, 샘플 세계관, 출력용 대체 글꼴 |
| `tests/` | 화면 도우미와 IPC 명령 연결 검사 |
| `src-tauri/src/tests.rs` | 저장·복원·검색·가져오기·출력 통합 테스트 |

원래의 `.svelte`/TypeScript 프로젝트와 프런트엔드 패키지 잠금 파일은 복구되지 않았습니다. `recovered-frontend`는 그 원본 소스가 아니라 배포 산출물입니다. 이 구분을 유지하기 위해 새 코드와 복구한 파일을 분리했습니다.

## 저장 방식과 성능 개선

- 본문은 Markdown, 구조는 SQLite에 저장합니다. DB 커밋 후 파일 기록이 중단되면 다음 실행에서 대기 중인 기록을 재개합니다.
- 변경이 없는 파일 저장을 생략하고, 대기 작업이 없는 조회에서 불필요한 쓰기 트랜잭션을 만들지 않습니다.
- 검색은 SQLite FTS5를 사용하며 한글·중국어·일본어 문서를 다룹니다.
- 화면의 엔티티 조회는 목록별 인덱스를 재사용합니다. 지도 픽셀 갱신은 32비트 팔레트 복사를 사용합니다.
- 미디어 참조를 조사할 때 문서마다 한 번씩 읽고 여러 파일 경로를 함께 찾습니다.
- IPC 작업은 UI 스레드 밖에서 실행됩니다. 하나의 세계관에 대한 변경은 직렬화됩니다.

앱 전체가 원본보다 몇 배 빨라졌다는 주장은 하지 않습니다. 성능은 데이터 크기와 기능에 따라 달라지며, 도우미 단위 측정은 전체 앱 성능과 다릅니다.

## 권리 및 기여

재구현 코드에 대한 프로젝트 라이선스는 소유자가 결정할 수 있도록 지정하지 않았습니다. 외부 구성 요소의 라이선스는 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)와 `licenses/`에 있습니다. 공개 저장소에 올릴 때 프로젝트 자체의 라이선스를 별도로 선택할 수 있습니다.
