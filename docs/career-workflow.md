# 지원 결과에서 다음 문서까지

지원 결과를 현재 대화에 알려주면 같은 호출자가 복기 → 반영 검토 → 작성 기준 확인·갱신 → 새 문서 확인을 이어간다. 실행자가 끝나면 진행을 보존하고 다음 명시 요청에서 재개한다. 별도 모델 실행기·상주 워커·작업 큐·자동 재시도를 추가하지 않는다.

CLI·저장·레코드 형식은 [career_workflow.rs](../src/career_workflow.rs), HTTP 접근은 [api.rs](../src/api.rs), 화면은 [CareerWorkflow.tsx](../web/src/CareerWorkflow.tsx)가 소유한다. 이 문서는 호출 순서와 연결·검증 방법을 설명한다. 개인 사실·작성 기준의 본문이나 사용자별 연결 설정은 소유하지 않는다.

## 연결과 검증

기존 ontology checkout과 PostgreSQL native store를 사용한다. 원본은 이 저장소의 검토된 commit이며 설치본은 기존 [로컬 설치 계약](local-operations.md#mac-로그인-후-온톨로지-자동-실행)으로 실행 파일·웹 번들을 복사한다. 기존 wrapper는 `career`를 그대로 전달한다. 설치 갱신 후 `local_service.py status`로 현재 빌드와 설치본의 일치 및 실제 앱 조회를 확인한다. `/career`와 지도의 `지원 작업공간` 링크에서 접근한다.

새 세션의 진입은 현재 native personal index와 작성 라우터가 소유한다. 그곳의 지원 결과 경로가 이 문서를 가리키도록 owner-scoped Harness로 연결한다. 저장소의 AGENTS는 공개 실행 계약만 연결하며 private 정책을 대신하지 않는다. 새 세션은 현재 정책을 완전히 읽고 기존 지원 상태 owner·입력 보관 경계·문서별 작성 기준을 선택한다.

`bash scripts/verify.sh`는 별도 임시 PostgreSQL에서 기존 원문·identity·history 보존, 실제 Core 적용·CLI 재개·API 보호 및 웹 검증을 수행한다. 합성 입력의 실제 새 대화 검증은 별도로 실행한다. 초기 fixture와 scripted role 구조 테스트의 성공을 실제 LLM 복기·작성·독립 검토 성공으로 대신하지 않는다. 운영 DB의 017 migration은 [백업·별도 빈 DB 복원·업그레이드](local-operations.md#기존-db-업그레이드) 확인 뒤 적용한다.

## 현재 대화의 호출 순서

각 Native 쓰기의 objective·완료 조건은 이번에 작성할 정확한 기록이나 기준 원문으로 한정한다. 예를 들어 반영 검토 기록의 완료 조건은 항목별 판단과 그 근거를 검토해 저장하는 것이다. 후속 기준 적용·새 문서 검증은 바깥 career 흐름의 다음 단계이며, 아직 수행하지 않은 효과를 이번 기록의 완료 사실로 요구하거나 주장하지 않는다.

prepare 후에는 역할별 입력을 확인한다. 이번 판단에 필요한 결과·복기·검토·현행 기준의 정확한 원문이 해당 Writer·Verifier·Reviewer에 전달되는지 확인하고, 누락은 기존 계약의 허용된 context·evidence source 선택으로 보완한다. authority나 source version이 결합돼 있다는 사실만으로 모든 역할에 그 원문 본문이 전달됐다고 판단하지 않는다. 이 확인은 기존 승인 scope를 확대하지 않는다. 실행·종료 유예·close 한도도 기존 유한 lifecycle 계약에서 실제 실행기가 처리할 수 있는 값으로 결합한다.

1. 기존 지원 상태 owner에 결과를 반영하고 최신 원문을 읽는다. 서류·면접 결과와 사유 공개 여부를 확인하며 미확인 원인은 그대로 둔다. 동일 지원·동일 결과에는 같은 `application_key`·`outcome_key`를 사용한다. 이 문자열은 caller가 기존 원문에서 식별한 사건의 키이며 모듈이 지원 결과의 의미를 추정하지 않는다.
2. `context identity`의 실제 store ID와 `read-documents`의 material identity·revision·현재 `source_digest`로 `SourceVersion` 참조를 만든다. 현재 요청에서 사용할 작성 기준을 결합해 `career start`를 호출한다. 조회는 schema를 자동 생성·적용하지 않는다.
3. 당시 공고·확인된 제출 자료·현재 경력 원문을 확인해 복기를 작성한다. 확인된 사실, 해당 제출 문서의 관찰, 가설, 미확인을 나눈다. agent가 작성한 보존 기록은 기존 Native Harness를 거친다. 실제 원문 적용 뒤 복기 참조를 `record`로 결합한다.
4. 모든 finding을 반영·보류·기각·기존 기준으로 해결 중 하나로 검토한다. 적용·기존 기준 해결은 결합한 기준의 경로를, 보류는 다시 검토할 조건을 지정한다. 가설·미확인 원인만으로 기준 변경을 채택하지 않는다. 사용자 의견과 독립 검토를 반영해 native 검토 기록을 만든 뒤 `record`로 결합한다.
5. 채택된 변경이 있으면 기존 Core로 정확한 기준 변경을 prepare한다. `bind-apply`에 run ID와 prepared digest를 먼저 결합한 뒤 같은 run을 적용한다. Native 적용의 `finalized` batch·typed Core bytes·실제 commit receipt·현재 bytes·immutable history를 `confirm-apply`로 대조한다. 검토가 채택한 target 집합과 실제 변경 집합이 달라지면 진행을 완료하지 않는다. 변경이 없으면 기존 기준을 유지하고 출력 확인으로 간다.
6. 최신 기준과 현재 전체 경력 후보 비교를 사용해 요청된 자소서·경력기술서·포트폴리오를 작성한다. Native career artifact는 기존 canonical comparison·composition 절차도 따른다. 최신 문서와 기준을 별도 문맥에서 독립 검토한 `output-review` 원문을 만든다. 그 참조와 모든 요청 surface의 확인을 `verification` 원문에 결합하고 `record`한다.

예를 들어 다음 JSON을 `bash scripts/brain.sh career`의 stdin으로 전달한다. 실제 값은 현재 store에서 읽은 값만 사용한다.

```json
{
  "op": "start",
  "store_id": "00000000-0000-4000-8000-000000000001",
  "application_key": "fictional-ai-ops",
  "outcome_key": "screening-rejected-1",
  "outcome": {
    "logical_path": "vault/personal/decisions/fictional-result.md",
    "state": {"state": "live", "material_id": "00000000-0000-4000-8000-000000000002", "revision": 1, "content_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
  },
  "criteria": [{
    "logical_path": "vault/personal/writing/fictional-criteria.md",
    "state": {"state": "live", "material_id": "00000000-0000-4000-8000-000000000003", "revision": 1, "content_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
  }],
  "surfaces": ["application-essay", "career-description", "portfolio"]
}
```

`list`는 store ID를, `get`은 store ID와 flow ID를 받는다. 변경은 `id`, `store_id`, 최신 `expected_revision`을 받는다. `record`는 native `SourceVersion` 하나를 추가한다. `bind-apply`는 `run_id`·`prepared_run_digest`를 추가한다. `confirm-apply`, `pause`, `resume`에는 추가 payload가 없다. 현재 입력을 다시 결합하는 `rebase`는 최신 `outcome`·`criteria`를 받고 복기부터 다시 검토한다. 이미 apply run을 결합했거나 완료한 흐름은 rebase로 초기화하지 않는다.

## Native 기록의 구조화 절

실제 근거·판단·문구는 native Markdown 원문에 둔다. 그 안의 단일 `career-workflow` fenced JSON은 다음 구조를 따른다. 상세 필드와 입력 한도는 코드의 typed 계약이 소유한다. 아래는 축약 설명이며 그대로 제출하는 payload가 아니다.

| kind | 연결할 내용 |
| --- | --- |
| `reflection` | 사건 키, finding ID·종류·statement와 실제 native evidence 참조 |
| `review` | 정확한 reflection 참조, 모든 finding의 disposition·reason·target·필요한 revisit 조건 |
| `output-review` | 정확한 review 참조, surface, 최신 criteria 참조, 선택적인 native artifact 참조, 독립 검토의 result와 observations |
| `verification` | 정확한 review와 최신 criteria 참조, 요청된 모든 surface의 artifact·independent review 참조 및 result |

외부 파일의 문서 관찰은 현재 요청에서 직접 검토하며, 보존된 native 근거가 없으면 reflection의 evidence는 빈 배열로 둔다. 확인된 경력 사실은 실제 native evidence를 결합한다. 외부 파일 자체는 현재 요청에서 직접 검토한다. 외부 출력의 `artifact`는 `null`로 두고 검토 기록에 기준과 surface·결론을 연결한다. 입력 binary·full visible copy·hash·filename·실제 경로·파일 pointer를 진행 DB에 넣지 않는다. 사용자가 보존한 native 텍스트 artifact만 참조하며 당시 제출 history를 새 작성문으로 덮어쓰지 않는다. 새 영구 경험 registry나 case별 capsule seed를 만들지 않는다.

Native artifact를 참조하면 그 원문의 실제 finalized 적용과 작성 시 최신 criteria의 source binding도 검사한다. 외부 문서는 독립 검토 기록의 정확한 기준·surface 결합을 검사한다. 이 구조 검사와 의미 검토는 다른 근거다. 레코드의 `passed` 자체가 모델의 주의·독립성·문서 품질을 보장하지 않으므로 caller는 기존 독립 리뷰 경계를 실제로 지켜야 한다.

## 재개와 완료

`source-drift`이면 현재 원문과 기존 판단을 다시 검토한다. 미결합 흐름은 최신 입력으로 rebase하고 복기부터 이어간다. 결합된 apply는 같은 Core run의 head·실제 효과를 먼저 확인한다. 모듈은 Core head와 apply lifecycle을 별도로 복제하거나 알 수 없는 실행을 자동 재시도하지 않는다. Native pending/committed apply는 기존 [복구 계약](local-operations.md#백업과-복원-확인)으로 처리하고 실제 finalized 후 확인을 재개한다. 결합된 run을 바꾸거나 제거해 복구 증거를 우회하지 않는다.

화면의 검토 입력은 현재 대화에 전달하기 전까지 브라우저 메모리의 초안이다. 같은 원문을 새로 확인하거나 중단·재개해도 유지하지만 페이지를 닫으면 보존되지 않는다. 모듈은 시작과 중단·재개 상태, 실제 적용된 Native 기록 참조와 확인된 각 단계의 진행을 보존한다. 화면 초안 자체를 완료된 검토로 보존하지 않으며, 전체 완료는 실제 기준 확인과 모든 요청 출력의 확인 뒤에 판정한다.

복기 검토 완료, 작성 기준 유지·갱신 확인, 새 문서 확인을 구분한다. `complete`는 해당 사건의 요청 surface를 모두 확인한 상태다. 다음 실제 지원에서 사용했거나 취업 성과가 개선됐다는 뜻으로 확대하지 않는다.
