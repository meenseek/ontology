# 개인 통합 온톨로지

사용자 한 명의 학습, 프로젝트, 개인 사업과 여러 조직의 맥락을 연결하는 로컬 앱이다. 원문 자료와 직접 기록을 함께 찾고 저장된 관계를 3D 지식 지도에서 탐색한다. 내용만 저장할 수 있으며 종류·묶음·연결은 필수가 아니다.

## 시스템 구성

| 구성요소 | 책임 | 구현 진입점 |
| --- | --- | --- |
| PostgreSQL native store | Markdown 원문 bytes·frontmatter·첨부, material identity·origin·revision·history 보존 | [native context](src/native_context.rs), [store](src/store.rs) |
| Context Core와 Harness | 검토된 원문 변경의 역할·검증·수락·적용·복구 계약 | [context-core](crates/context-core/src/lib.rs), [native adapter](src/native_harness.rs) |
| 조회 projection | 원문에서 검색·의미 관계·앱 조회 사본을 파생하고 현재 revision과 연결 | [context projection](src/context_projection.rs), [app importer](src/context_importer.rs) |
| 기억과 정제 | 직접 기록·목적 소속·이력·근거 보존, 독립 검토한 정리 결과의 원자적 반영 | [memory](src/memory.rs), [curation](src/curation.rs) |
| 로컬 HTTP·CLI | 같은 저장·조회 기능에 대한 화면과 native 호출 경계 | [HTTP](src/main.rs), [CLI wrapper](scripts/brain.sh) |
| Web 지도와 상세 | 3D 탐색·목록·원문 읽기·허용된 수동 편집 | [App](web/src/App.tsx), [graph model](web/src/graph.ts) |

HTTP 서버와 PostgreSQL은 `127.0.0.1`에만 연결한다. 설치된 로컬 앱은 `47831`, 개발용 앱은 `47832`, DB는 `55432` 포트를 사용한다. 네트워크 배포나 OS 사용자 사이의 인증을 제공하지 않는다. Native 원문 조회에는 HTTP 서버가 필요하지 않다.

## 원문과 조회 사본

지식·규칙의 원문과 첨부는 PostgreSQL native store가 소유한다. 같은 material identity 아래 revision과 history를 남기며, 최초 origin과 원문 digest는 수정 뒤에도 보존한다. Native `Create`로 만든 자료를 외부에서 가져온 자료로 표시하지 않는다. 이전 filesystem 사본을 다시 가져와 native 수정을 덮어쓰지 않는다.

Git 문서의 원본 소유권은 원래 Git 저장소에 남는다. 허용한 로컬 저장소의 확정 커밋과 정확한 경로에서 앱 조회 사본을 가져온다. Native 자료는 허용한 store·scope·경로의 원문에서 조회 사본을 만든다. 검색·ontology·앱 조회 사본은 모두 파생 projection이며 현재 원문·revision·digest와 결합한다. Projection 재생성은 원문·material ID·revision·이력을 바꾸지 않는다.

Native scope는 `profile`, `personal`, `work/common`, `work/<회사 slug>`다. 앱의 `personal`·`meenseek` 조회 범위와 구별하며, 앱에서 보이는 위치가 원문의 소속을 바꾸지 않는다. 범위를 지정한 읽기와 보존 경계는 [AGENTS.md의 native 원문 조회](AGENTS.md#native-원문-보존과-조회)를 따른다.

## 데이터 흐름

1. Git의 확정 커밋 또는 native의 정확한 원문을 읽고 source identity와 digest를 보존한다.
2. 검색·의미 관계·앱 조회 projection을 만들고 원문 revision에 연결한다. 부재·실패·미준비 상태를 구별하며, 이전 조회 사본을 현재 원문으로 대신하지 않는다.
3. 로컬 API가 현재 범위의 문서·기록·관계를 반환하고 Web이 지도와 상세를 표시한다.
4. 직접 기록은 같은 DB에 저장한다. 정제 지식은 같은 범위의 원문·기존 지식과 대조하고 독립 검토한 후보만 근거와 처리 이력에 함께 반영한다.

지정 원문 갱신은 명시한 설정의 정확한 경로만 처리한다. 다른 대화나 등록되지 않은 폴더를 자동 수집하지 않는다. 제품 실험·측정·제작과 그 판정은 기존 소유 도구에 남는다.

## 저장·검토·근거

사용자가 직접 작성한 기존 원문의 허용된 수동 편집은 현재 revision·digest로 충돌을 확인하고 새 버전·이력을 저장한다. 에이전트가 작성·수정한 native 원문은 Context Core의 검토·적용 경계를 사용한다. 수동 저장을 Core 검토·수락으로 표시하지 않는다.

기억의 `accepted`는 저장 상태이며 진실 판정이 아니다. 출처 현재성과 인용 일치 검사는 의미 검증이나 reviewer 독립성의 증명이 아니다. 정제 작성과 독립 검토는 native 작업자가 수행하고 앱은 입력·근거 검증과 원자적 저장을 담당한다. 별도 유료 모델 API·벡터 DB·앱 내부 실행기를 사용하지 않는다.

과거 근거 본문은 범위별 bytes와 이력 참조로 보존한다. Git·native 원문이 바뀌거나 없어져도 보존된 당시 근거를 읽을 수 있다. 보존하지 않은 과거 본문은 복원 가능한 것으로 표시하지 않는다. 전체 native 원문·첨부·이력·기억·근거의 복구에는 DB 전체 백업이 필요하며 Git 커밋이나 digest만으로 bytes를 복원할 수 없다.

스키마 초기화는 적용된 baseline과 migration의 이름·SQL digest를 검사한다. Git은 스키마 변경 이력을, DB의 `ontology_migrations`는 해당 DB의 실제 적용 상태를 소유한다. 적용된 SQL과 DB 적용 기록은 검사와 복원을 위해 보존한다.

## 3D 지식 지도

기본 화면은 **3D 지도**, 기본 묶음 기준은 **저장된 목적 소속**이다. 사용자가 목록으로 전환하거나 3D 화면에 실패하면 목록을 표시한다. 저장된 목적 소속은 관계·검색 변화와 별도로 유지하며, 관계 군집·화면상 근접 요약·폴더 구조와 구별한다.

문서 링크와 의미 개체 관계, 기록의 출처 근거는 서로 다른 관계다. 지도상의 거리나 같은 태그만으로 같은 목적·사실 관계를 추론하지 않는다. 상세 정의와 소속 보존 계약은 [관계 taxonomy와 안정적인 그룹](docs/graph-taxonomy.md)이 소유한다. [개인 기억 판단](docs/personal-memory-grouping.md)과 [신규 문서 판단](docs/document-grouping.md)은 각각의 자동 분류 기준을 소유한다.

## 문서와 정책의 소유권

이 README는 시스템 구성·데이터 흐름·저장 및 실행 경계를 설명한다. [AGENTS.md](AGENTS.md)는 저장소 개발, 로컬 설치·실행·복구·백업·검증과 호출 참고를 소유하며 [문서별 소유권](AGENTS.md#이-저장소의-문서-소유권)을 안내한다. 조직 저장소의 공통 문서 배치 기준은 `meenseek/.github`의 `docs/repository-rules.md`에 둔다.

사용자별 source 선택, 연결 정보와 정책은 Git에 포함하지 않는다. 이 저장소는 scoped source 읽기와 identity·revision·digest 보존, 검토·적용·복구의 실행 계약을 소유한다. 호출자의 승인된 설정과 현재 원문으로 동작하며, 조회 사본이나 저장소 문서가 그 승인을 대신하지 않는다.

Harness의 설치별 정책 선택은 명시적 caller 설정으로 전달한다. 누락된 설정과 기존 plan에 결합된 설정 digest의 변경은 source 접근 전에 거절한다. 형식과 CLI 호출은 [호출자 정책 설정](AGENTS.md#호출자-정책-설정)이 소유한다.

원문 이동과 실행 상태 경로 이동은 현재 지원하지 않으며 표시 명칭 변경과 구별한다.
