# 1.9.0-rebuilt.1 검증 기록

검증일: 2026-11-14. Windows x64, Rust 1.100.0 GNU, Node.js 24 환경에서 검사했습니다.

| 항목 | 결과 |
| --- | --- |
| Rust 통합 테스트 | 13개 통과, 실패 0개 |
| 외부 원본 샘플 전용 테스트 | 1개 기본 제외; 내장 샘플은 별도 테스트로 검증 |
| Node 테스트 | 5개 통과 |
| 화면 명령 등록 검사 | 183개 누락 없음 |
| Clippy | 모든 타깃에서 경고를 오류로 처리해 통과 |
| Rustfmt | 코드 포맷 적용 및 확인 |
| PDF 출력 | 한글 텍스트 추출·페이지 수 검사, 렌더링한 2페이지 육안 확인 |
| 릴리즈 빌드 | `cargo build --release --locked` 통과, Windows x64 GUI 실행 파일 생성 |
| 실행 파일 의존성 | Windows 시스템 DLL과 동봉 WebView2Loader.dll 확인; 별도 MinGW DLL 불필요 |

테스트는 임시 세계관을 만들어 저장·검색·복원·파일 기록 중단 복구·가져오기·내보내기를 검사합니다. 실제 IPC 검사는 Tauri MockRuntime을 사용하므로 Windows 창을 만들지 않습니다. 전체 WebView2 GUI를 사용한 모든 기능의 검증을 대신하지는 않습니다.

## 성능 측정의 범위

Node 도우미 테스트는 동일한 결과를 내는지 확인하면서 기존 선형 조회와 캐시 인덱스, 기존 픽셀 루프와 32비트 팔레트 복사를 비교합니다. 실행 시 중간값이 출력되며 하드웨어와 부하에 따라 달라집니다. 앱 전체의 시작 시간, 메모리 사용량 또는 편집 프레임 속도 개선율은 측정하지 않았습니다.

## 다시 검사하기

```powershell
node --test tests/*.mjs
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
./tools/build.ps1 -Action test
./tools/build.ps1 -Action check
./tools/build.ps1 -Action release
```

재현에 필요한 코드, 데이터 템플릿, 원본 스키마 이력, 복구 화면, Cargo.lock은 소스 ZIP에 포함됩니다. 알려진 차이는 [COMPATIBILITY.md](COMPATIBILITY.md)를 참조하세요. GitHub Actions 정의는 포함했지만 원격 실행 결과는 아직 없습니다.
