# Ontology repository instructions

These instructions own task scope and document routing for this repository. Detailed development, installation, execution, recovery, backup and verification contracts live in [docs/local-operations.md](docs/local-operations.md). Use the checkout containing this file as the repository root. Follow repository-local contracts and the exact task scope; reading this guide grants no source access, writes, migrations or recurring work.

A repository-only review or development task uses this repository and its synthetic test inputs. Do not discover external source materials or configure a user's store to satisfy an ordinary code task. For an explicitly requested operation on an existing installation, use the caller's approved store identity and exact source selection; stop that dependent operation if they are unavailable.

## 이 저장소의 커밋

이 ontology 저장소의 커밋 메시지는 제목과 본문 모두 한국어를 기본으로 쓴다. 사용자가 다른 언어를 명시하면 그 지시를 따른다.

## 이 저장소의 문서 소유권

공통 배치 기준은 `meenseek/.github`의 `docs/repository-rules.md` 「문서 소유권」 절이다. 실제 접근 가능한 `.github` checkout에서 읽으며 로컬 절대 경로를 추측하지 않는다. 공통 배치 기준이나 문서 소유권을 바꿀 때 그 절을 직접 읽는다.

| 문서 | 소유하는 내용 |
| --- | --- |
| [README.md](README.md) | 시스템 아키텍처·구성요소·데이터 흐름·저장 및 실행 경계 |
| 이 AGENTS.md | 저장소 작업 범위·문서 소유권·작업별 읽기 진입점 |
| [로컬 실행·복구·검증](docs/local-operations.md) | 로컬 개발·설치·실행·복구·백업·검증과 호출 참고 |
| [관계 taxonomy](docs/graph-taxonomy.md) | 자료 관계·그룹·폴더의 의미, 목적 소속 보존·변경과 진단의 계약 |
| [개인 기억 판단](docs/personal-memory-grouping.md) | 개인 기억의 기존 묶음 배정·후보·미분류 판단 |
| [신규 문서 판단](docs/document-grouping.md) | 신규 문서의 정의된 목적 배정·후보·미분류 판단 |
| [종합평가](docs/evaluation.md) | 제품의 종합평가 항목·지표 정의·근거 범위와 효과 비교 기준 |
| `notes/*.md` | Git이 소유하는 개별 원문 자료. 실행 지침이나 공통 정책으로 취급하지 않는다. |

타입·설정·입출력·저장 동작은 각 절에서 지목하는 코드가 소유한다. 문서의 예시는 그 계약을 설명한다. 이 문서는 저장소의 계약을 설명하며 사용자별 정책·연결 정보를 정의하지 않는다.

## 종합평가

제품 종합평가 또는 평가 기준·지표의 검토를 요청받으면 [종합평가 기준](docs/evaluation.md)을
읽고 적용한다. 일반 원문 조회·사소한 수정에서는 이 문서를 기본 입력에 추가하지 않는다.
평가 기준을 읽는 것과 실험·운영 검증을 실행하는 것은 구분하며, 실제 실행은 요청 범위와
[상세 실행·검증 계약](docs/local-operations.md)을 따른다.

아래에서 현재 작업에 필요한 절만 선택한다. 이전 진입점의 절 링크는 유지하며, 상세 계약은 연결된 문서가 소유한다.

## 로컬 연결

[상세 계약](docs/local-operations.md#로컬-연결).

## 실행과 종료

[상세 계약](docs/local-operations.md#실행과-종료).

### 기존 DB 업그레이드

[상세 계약](docs/local-operations.md#기존-db-업그레이드).

### 앱 시작과 종료

[상세 계약](docs/local-operations.md#앱-시작과-종료).

### 이전 설치 이름에서 데이터 옮기기

[상세 계약](docs/local-operations.md#이전-설치-이름에서-데이터-옮기기).

### 연결 상태와 복구

[상세 계약](docs/local-operations.md#연결-상태와-복구).

### Mac 로그인 후 온톨로지 자동 실행

[상세 계약](docs/local-operations.md#mac-로그인-후-온톨로지-자동-실행).

### 다른 Mac에서 같은 맥락 사용

[상세 계약](docs/local-operations.md#다른-mac에서-같은-맥락-사용).

## 3D 지식 지도에서 탐색하기

[상세 계약](docs/local-operations.md#3d-지식-지도에서-탐색하기).

### 검색과 화면 초점

[상세 계약](docs/local-operations.md#검색과-화면-초점).

## 지도와 목록에서 원문 찾기

[상세 계약](docs/local-operations.md#지도와-목록에서-원문-찾기).

## 기억 저장과 다시 찾기

[상세 계약](docs/local-operations.md#기억-저장과-다시-찾기).

## 자동 정리와 결과 반영

[상세 계약](docs/local-operations.md#자동-정리와-결과-반영).

### native 작업자가 한 건 처리하는 순서

[상세 계약](docs/local-operations.md#native-작업자가-한-건-처리하는-순서).

### 실험과 제작 도구 연결

[상세 계약](docs/local-operations.md#실험과-제작-도구-연결).

## Git 문서 가져오기

[상세 계약](docs/local-operations.md#git-문서-가져오기).

## Native context를 앱으로 가져오기

[상세 계약](docs/local-operations.md#native-context를-앱으로-가져오기).

## Native 원문 보존과 조회

[상세 계약](docs/local-operations.md#native-원문-보존과-조회).

### 경력 경험 후보와 전체 비교

[상세 계약](docs/local-operations.md#경력-경험-후보와-전체-비교).

### 최초 자료 반입

[상세 계약](docs/local-operations.md#최초-자료-반입).

### 검토된 원문 변경

[상세 계약](docs/local-operations.md#검토된-원문-변경).

### 호출자 정책 설정

[상세 계약](docs/local-operations.md#호출자-정책-설정).

### 하네스 비교 실험

[상세 계약](docs/local-operations.md#하네스-비교-실험).

### 사용자가 직접 작성한 원문 저장

[상세 계약](docs/local-operations.md#사용자가-직접-작성한-원문-저장).

## 지정한 원문 자동 갱신

[상세 계약](docs/local-operations.md#지정한-원문-자동-갱신).

## 화면에서 문서 읽고 분류·연결하기

[상세 계약](docs/local-operations.md#화면에서-문서-읽고-분류연결하기).

## 백업과 복원 확인

[상세 계약](docs/local-operations.md#백업과-복원-확인).

## 개발 검증

[상세 계약](docs/local-operations.md#개발-검증).
