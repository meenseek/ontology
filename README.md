# meenseek ontology

사실·결정·선호·아이디어를 기억으로 저장하고 필요할 때 다시 찾는 단일 사용자용 로컬 앱이다. Git에 커밋된 문서와 명시적으로 허용한 Vault 문서도 조회 사본으로 가져와 검색·분류·연결하고 기억의 근거로 선택할 수 있다.

새 기억, 범위별 기억 묶음, 분류·연결·이력은 PostgreSQL이 소유한다. Git·Vault 원본은 원래 도구에 남는다. 앱은 Vault Markdown을 이전하거나 쓰지 않으며, 다른 대화를 자동으로 수집하거나 사실을 스스로 확정하지 않는다.

HTTP 서버와 PostgreSQL은 `127.0.0.1`에만 연결하며, 네트워크 배포나 OS 사용자 사이의 인증을 제공하지 않는다. Workbench·Observatory·Factory 연동, MCP, LLM, 벡터 DB, 자동 작업 큐는 아직 지원하지 않는다.

## 실행과 종료

현재 검증 환경은 Rust **1.98.1**(`rust-toolchain.toml`에 고정), Node **26**, pnpm **11.8.0**, Docker Compose, Git, OpenSSL, 설치된 `cargo-audit`다. Docker가 실행 중이어야 한다.

앱 루트에서 기존 DB와 비밀번호가 일치하는 로컬 `.env`를 사용한다. `ONTOLOGY_DB_PASSWORD`는 영문 대소문자·숫자·`_`·`-`로 된 16~128자다. `.env`는 Git에 넣지 않는다.

### 기존 DB 업그레이드

기존 DB를 사용하는 버전 업그레이드에서는 새 서버를 시작하기 전에 다음 순서를 따른다.

1. `Ctrl-C`로 기존 HTTP 서버와 자동 갱신을 종료하고, importer·`sync-once`·기억 CLI 등 모든 DB 쓰기를 중단한다. PostgreSQL은 실행 상태로 둔다.
2. 아래 [백업과 복원 확인](#백업과-복원-확인)을 실행해 전체 백업을 보관하고, **별도의 빈 DB**에서 복원·업그레이드와 건수 보존을 확인한다.
3. 복원 확인이 성공한 뒤 다음 명령으로 운영 DB에 `init`을 실행한다. 적용된 baseline과 migration을 검사하고 아직 적용하지 않은 migration을 반영한다.

```bash
cd /Users/meenseek/Desktop/meenseek-ontology
set -a
source .env
set +a
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
cargo run --locked -- init
```

운영 `init`까지 성공한 뒤 아래 앱 시작 명령으로 새 서버를 실행한다. 새 버전의 서버와 CLI도 migration을 검사·적용하므로 **업그레이드가 끝나기 전에 실행하지 않는다.** 어느 단계든 실패하면 DB 쓰기를 재개하기 전에 원인을 확인한다.

### 앱 시작과 종료

```bash
cd /Users/meenseek/Desktop/meenseek-ontology
chmod 600 .env
set -a
source .env
set +a
bash scripts/dev.sh
```

**기존 DB 볼륨이 없는 새 설치에서만**, 위 명령 전에 다음으로 `.env`를 만들 수 있다. 기존 `.env`는 덮어쓰지 않는다.

```bash
(
  umask 077
  set -o noclobber
  printf 'ONTOLOGY_DB_PASSWORD=%s\n' "$(openssl rand -hex 24)" > .env
)
```

`scripts/dev.sh`는 PostgreSQL **18.4**를 `127.0.0.1:55432`에 시작하고, 웹 의존성 설치와 번들 생성 뒤 [로컬 앱](http://127.0.0.1:47831)을 실행한다. 터미널은 서버가 실행되는 동안 사용 중이다. DB 이름과 사용자는 `ontology`, 영구 볼륨 이름은 `meenseek-ontology-data`다.

같은 볼륨을 다시 사용할 때는 **같은 비밀번호**가 필요하다. `.env`나 컨테이너 환경변수에 새 비밀번호를 넣어도 기존 DB 비밀번호는 바뀌지 않는다. 기존 볼륨이 있는데 `.env`를 잃었다면 새 비밀번호를 생성하지 말고 기존 값을 복구한다.

`Ctrl-C`로 HTTP 서버를 종료한다. DB도 멈추려면 같은 앱 루트에서 다음을 실행한다. 데이터는 보존되며, 다음 `bash scripts/dev.sh` 실행 때 다시 사용한다.

```bash
docker compose stop postgres
```

## 기억 저장과 다시 찾기

브라우저의 첫 화면에서 `meenseek` 또는 `개인` 범위를 고르고 기억을 저장한다. 제안 수락, 정정, 철회, 잊기, 이력 조회와 기억 묶음 관리도 이 화면에서 처리한다. 원문 검색·분류·연결은 별도 문서 화면에서 이용한다. UI·CLI·API는 같은 기억 처리 규칙을 사용한다.

문서 태그(`topics`)는 문서마다 선택적으로 최대 10개 붙일 수 있다. 기억은 같은 범위의 기억 묶음(`subjects`) 하나를 `subject_id`로 선택하거나 소속 없이 둘 수 있다. 문서 태그와 기억 묶음은 서로 별도의 분류다.

에이전트나 터미널에서는 `bash scripts/brain.sh`에 JSON 객체 하나를 표준 입력으로 보낸다. 실행 중인 DB가 필요하며 HTTP 서버는 없어도 된다. 스크립트는 앱 루트로 이동하고, 명시한 `DATABASE_URL`이 없으면 신뢰하는 기존 로컬 `.env`의 `ONTOLOGY_DB_PASSWORD`로 DB 주소를 만든다. 내부적으로 `cargo run --quiet --locked -- brain`을 실행하며, 표준 출력은 JSON이고 오류 시 종료 코드는 0이 아니다.

아래 내용은 **바꿔 쓸 예시**다. 실행하면 지정된 DB에 실제로 저장된다. 작은따옴표를 붙인 heredoc으로 JSON을 전달해 본문에 셸 변수나 명령 치환이 적용되지 않게 한다.

```bash
cd /Users/meenseek/Desktop/meenseek-ontology
bash scripts/brain.sh <<'JSON'
{"op":"remember","scope":"personal","idempotency_key":"readme_example_001","memory":{"kind":"preference","title":"답변 형식 예시","body":"답변은 핵심을 먼저 짧게 정리한다."}}
JSON

bash scripts/brain.sh <<'JSON'
{"op":"recall","scope":"personal","query":"답변","limit":5}
JSON
```

`scope`는 `meenseek` 또는 `personal`, 기억의 `kind`는 `fact`, `decision`, `preference`, `idea`다. `remember`는 사용자 기록(`accepted`/`user`), 같은 입력 형태의 `propose`는 에이전트 제안(`proposed`/`assistant`)을 만든다. **`accepted`는 사용자가 보관하기로 했다는 뜻이지 사실 검증 완료가 아니다.** 제안을 수락하거나 정정해도 작성 주체는 유지한다.

생성 키 `idempotency_key`는 영문·숫자·`_`·`-` 8~128자다. 같은 키와 같은 내용으로 재시도하면 기억과 이력이 중복되지 않는다. 다른 내용을 보내면 충돌하고, 잊은 기억의 키를 재사용하면 `gone` 오류가 난다. 새 기억에는 새 키를 쓴다.

후속 요청에도 `op`와 `scope`를 넣는다. 기억의 `id`는 응답의 `m_UUID`, 변경 요청의 `revision`은 **현재 응답에서 받은 숫자**를 사용한다. 충돌하면 `read`로 다시 읽고 판단한다.

| `op` | 함께 보낼 필드와 동작 |
| --- | --- |
| `read`, `history` | `id`. 현재 내용 또는 이력 조회. `history`는 `before_revision`, `limit`으로 다음 페이지를 요청한다. |
| `list` | 선택적으로 `status`, `subject_id`, `query`, `after`, `limit`으로 목록을 좁힌다. |
| `correct` | `id`, `revision`, `memory`. 현재 기억 전체를 교체하고 이력을 남긴다. 작성 주체와 상태는 유지한다. |
| `accept` | `id`, `revision`. 제안만 수락한다. |
| `withdraw` | `id`, `revision`. 회상에서 제외하고 이력은 남긴다. 철회한 기억은 정정하거나 다시 수락할 수 없다. |
| `forget` | `id`, `revision`. 해당 기억·이력·근거 연결을 앱의 활성 데이터에서 논리적으로 삭제한다. |
| `subject-create`, `subjects` | 기억 묶음 생성은 `idempotency_key`, `name`; 목록은 선택적으로 `query`, `after`, `limit`. 반환된 `p_UUID`를 기억이나 조회의 `subject_id`로 쓴다. |
| `evidence` | 선택적으로 `query`, `limit`을 보내 같은 범위의 현재 근거 후보를 찾는다. |

`memory`에는 `kind`, `title`, `body`와 선택적으로 `subject_id`, `effective_from`, `effective_until`, `evidence`를 넣는다. 유효 시각은 UTC Unix 초이며 시작은 포함하고 끝은 제외한다. 생략할 선택 필드는 빼고, 응답 객체를 통째로 요청에 복사하지 않는다. 알 수 없는 필드는 거부한다. 입력 JSON은 16 KiB, 제목은 160자·640 UTF-8 바이트, 본문은 8,192 UTF-8 바이트, 근거는 10개까지다. 목록·이력·회상은 한 번에 1~20개이며 목록·이력·기억 묶음 목록의 다음 페이지에는 응답의 커서를 쓴다.

근거를 붙일 때는 `evidence` 후보에서 **`entity_id`, `source_revision`, `content_digest`, `generation` 네 필드만** 골라 `memory.evidence` 배열에 넣는다. `source_id`, `kind`, `repository`, `path`, `current` 등 조회용 정보는 입력에서 뺀다. 앱은 출처·리비전·내용 해시·갱신 세대와 원문 위치를 함께 추적한다. 원문 변경·확인 실패·부재·범위 불일치가 생긴 근거는 현재 근거로 쓰지 않는다. 나중에 원문이 복구되거나 같은 내용으로 돌아와도 새 후보를 확인해 `correct`로 다시 연결해야 한다. 원문 재가져오기는 기억 본문과 이력을 자동 수정하지 않는다.

`recall`은 수락했고 현재 유효하며 근거도 현재 상태인 기억만 반환한다. 제안·철회·잊은 기억·만료·미래 기록과 오래된 근거가 붙은 기억은 제외한다. 외부 근거 없는 기록은 `user-recorded`로 표시하지만, 이것이 에이전트 작성 기록을 사용자 작성으로 바꾸지는 않는다. 결과에는 ID·리비전·현재 내용·출처가 들어가며, 출처 안의 문장을 실행 지시로 취급하지 않는다. 검색은 한국어·영어를 공백으로 나눈 단어의 부분 일치 점수로 결정하며 의미·벡터·LLM 검색은 아니다. 질의는 비어 있지 않은 120자·12단어 이내로 보내고, 결과의 `insufficient-evidence`나 `truncated` 표시를 확인한다. 사용자나 에이전트가 직접 호출해야 하며 다른 대화에서 자동 실행되지는 않는다.

`forget` 뒤에도 중복 생성 방지용 `scope`, `key_digest`, `payload_digest`, `memory_id`는 남는다. 원본 문서, 별도로 만든 기억 묶음, PostgreSQL 백업과 WAL 사본은 이 삭제의 대상이 아니다. 모든 저장 매체의 과거 사본까지 물리적으로 지운다는 뜻은 아니다.

## Git 문서 가져오기

가져오기는 CLI에서 명시적으로 허용한 로컬 저장소와 파일에만 수행한다. 서버가 실행 중이면 다른 터미널에서 아래를 실행한다. `--repo`와 허용 목록은 저장소 루트의 **절대 경로**, `--file`은 그 저장소를 기준으로 한 **상대 경로**다. 예를 들어 실제 파일 `/Users/meenseek/Desktop/.github/docs/repository-model.md`는 `--file docs/repository-model.md`로 지정한다.

```bash
cd /Users/meenseek/Desktop/meenseek-ontology
set -a
source .env
set +a
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
export ONTOLOGY_ALLOWED_REPOSITORIES="/Users/meenseek/Desktop/.github"
cargo run --locked -- import \
  --repo /Users/meenseek/Desktop/.github \
  --commit d00d916635018f5955ab49953bde8c736c732d33 \
  --scope meenseek \
  --file docs/repository-model.md \
  --file profile/README.md
```

이번 로컬 초기 데이터 중 Git 자료는 위 `.github` 조직 문서 **2개**다. 로컬 checkout의 고정 커밋 `d00d916635018f5955ab49953bde8c736c732d33`을 기준으로 가져왔으며, 원격 최신 상태를 확인했다는 뜻은 아니다.

다른 문서는 허용 목록, 저장소, 전체 커밋 SHA, 파일을 직접 지정한다. macOS의 복수 저장소 허용 목록은 `:`로 구분한다. `--scope`는 `meenseek` 또는 `personal`이며 앱에서 개인 자료로 분류할 문서는 `personal`을 사용한다. 허용 목록에 들어간 저장소의 하위 디렉터리까지 자동 허용하지 않는다. 브랜치명·축약 SHA·절대 파일 경로·상위 경로·심볼릭 링크는 받지 않는다. 한 번에 중복 없는 `.md`, `.txt`, `.rst` 파일 1~100개, 파일당 최대 64 KiB의 UTF-8 텍스트를 처리한다. 작업 트리의 미커밋 내용과 원격 자료는 가져오지 않는다.

동일한 저장소·경로·scope를 다시 가져오면 Git 원문 사본과 출처 확인 정보를 갱신한다. 사용자가 저장한 기억·분류·연결·이력은 유지한다. 고정 커밋에서 파일이 사라진 경우에는 부재를 기록하고 마지막 원문을 보존한다. 읽기 실패 시에도 마지막 성공 자료를 보존하며 화면에 확인 실패를 표시한다.

## Vault 문서 가져오기

Vault 원본은 그대로 두고 공식 `read` 명령이 반환한 제목·본문을 앱의 조회 사본으로 저장한다. 먼저 기존 Vault 저장소에서 실행 파일을 빌드한다.

```bash
cd /Users/meenseek/Desktop/llm-context-vault
cargo build --locked -p llm-context-vault
```

`--vault-binary`는 위에서 빌드한 실행 파일의 **절대 경로**, `--vault-root`와 `ONTOLOGY_ALLOWED_VAULT_ROOTS`는 **Vault 데이터 루트의 절대 경로**다. 아래 예시는 프로젝트 목적 문서 한 개만 가져온다.

```bash
cd /Users/meenseek/Desktop/meenseek-ontology
set -a
source .env
set +a
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
export ONTOLOGY_ALLOWED_VAULT_ROOTS="/Users/meenseek/Desktop/llm-context-vault/vault"
cargo run --locked -- import-vault \
  --vault-binary /Users/meenseek/Desktop/llm-context-vault/target/debug/llm-context-vault \
  --vault-root /Users/meenseek/Desktop/llm-context-vault/vault \
  --vault-scope personal \
  --scope meenseek \
  --file projects/meenseek-ontology.md
```

`--vault-scope personal`은 읽을 위치인 `vault/personal`을 고르고, `--scope meenseek`는 앱에서 검색·분류·연결할 범위를 고른다. **Vault의 자료 범위와 앱의 분류 범위는 별개**다. `--file`은 Vault scope 안의 상대 Markdown 경로이므로 `personal/`을 붙이지 않는다. 앱에 표시되는 출처 경로는 `personal/projects/meenseek-ontology.md`다. 현재 초기 자료는 위 Git 조직 문서 2개와 이 Vault 프로젝트 목적 문서 1개다.

Vault 루트는 허용 목록 항목과 정확히 일치해야 하며, 하위 디렉터리를 자동 허용하지 않는다. macOS에서 복수 루트는 `:`로 구분한다. 한 번에 중복 없는 Markdown 파일 1~100개를 `--file`로 각각 지정한다. 상위 경로·와일드카드는 받지 않으며 Vault `read`의 파일 크기·경로·민감 자료 제외 규칙도 적용된다.

같은 Vault 루트·Vault scope·파일 경로·앱 scope로 다시 가져오면 조회 사본과 출처 확인 정보를 갱신하고, 사용자가 저장한 기억·분류·연결·이력은 유지한다. Vault 읽기가 실패하면 요청 전체의 새 사본을 반영하지 않고, 이미 등록된 요청 자료에는 확인 실패를 표시하며 마지막 성공 사본을 보존한다. **Vault 파일의 부재도 읽기 실패로 처리한다.** Git처럼 고정 커밋에서 확인한 부재 상태를 기록하지 않는다.

Vault가 민감값을 가린 제목·본문으로 조회 사본을 만들기 때문에 원문 SHA-256과 조회 내용 SHA-256은 구분한다. 원문 SHA-256은 가리기 전 파일의 해시이고, 조회 내용 SHA-256은 앱이 저장한 조회 사본의 해시다.

## 지정한 원문 자동 갱신

자동 갱신은 `ONTOLOGY_SYNC_CONFIG`를 설정해야 켜진다. 추적되는 형식 예시는 `sync.example.json`이며 실제 설정 파일 `sync.local.json`은 Git에서 제외된다. 아래는 초기 자료인 **정확히 세 경로**만 갱신하는 설정이다. 앱 루트의 `sync.local.json`에 저장하고, 위 Vault 실행 파일 빌드를 먼저 마친다.

```json
{
  "interval_seconds": 60,
  "sources": [
    {
      "kind": "git",
      "root": "/Users/meenseek/Desktop/.github",
      "scope": "meenseek",
      "ref": "HEAD",
      "paths": ["docs/repository-model.md", "profile/README.md"]
    },
    {
      "kind": "vault",
      "root": "/Users/meenseek/Desktop/llm-context-vault/vault",
      "scope": "meenseek",
      "vault_scope": "personal",
      "binary": "/Users/meenseek/Desktop/llm-context-vault/target/debug/llm-context-vault",
      "paths": ["projects/meenseek-ontology.md"]
    }
  ]
}
```

기존 DB의 업그레이드를 완료하고 기존 서버를 종료한 상태에서 다음을 실행한다. 한 번 갱신이 성공하면 같은 환경에서 서버를 시작해 자동 갱신을 이어간다.

```bash
cd /Users/meenseek/Desktop/meenseek-ontology
set -a
source .env
set +a
export DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/ontology"
export ONTOLOGY_SYNC_CONFIG="$PWD/sync.local.json"
cargo run --locked -- sync-once && bash scripts/dev.sh
```

이 설정 자체가 자동 갱신의 유일한 허용 목록이다. 수동 가져오기의 `ONTOLOGY_ALLOWED_REPOSITORIES`·`ONTOLOGY_ALLOWED_VAULT_ROOTS`는 수동 명령에 계속 필요하지만 자동 갱신에는 쓰지 않는다. 디렉터리·와일드카드로 파일을 열거하거나 원격 저장소를 가져오지 않는다. Git은 매번 지정한 로컬 `ref`를 전체 커밋으로 확정하며 미커밋 텍스트는 읽지 않는다. 따라서 위 `HEAD` 설정은 초기 고정 커밋 이후의 로컬 커밋도 반영한다. Vault는 지정한 파일의 공식 `read`만 호출한다.

설정은 최대 32 KiB, 원문 항목 1~8개, 전체 경로 100개, 간격 15~3,600초다. 읽기 전에 설정 전체를 검증하고, 잘못된 설정이면 자동 원문 읽기를 멈춘다. 파일을 고치면 다음 주기에 다시 읽는다. 실패는 설정 항목별로 격리되어 다른 항목은 계속 처리하며, 한 항목 안의 파일 묶음은 함께 반영한다. 실패한 조회 사본은 마지막 성공 내용을 보존하면서 확인 실패로 표시한다.

서버의 `serve`가 실행되는 동안 시작 시 한 번, 이후 지정 간격마다 설정과 원문 상태를 다시 읽는다. 재시작·절전 복귀 뒤에는 현재 상태를 확인하고 놓친 횟수만큼 몰아서 실행하지 않는다. macOS 로그인 시 자동 실행되는 데몬은 아니다. 자동 갱신을 끄려면 서버를 종료하고 `ONTOLOGY_SYNC_CONFIG`를 환경과 `.env`에서 해제한 뒤 다시 시작한다.

수동 가져오기와 자동 갱신·`sync-once`는 DB 잠금으로 동시 실행을 막는다. 겹치면 다른 작업이 끝난 뒤 재시도한다. `sync-once`는 항목별 종류·건수·호출량·오류 분류를 담은 JSON을 출력하고 실패나 중복 실행 시 0이 아닌 코드로 종료한다. 화면의 **갱신 상태 다시 읽기**는 마지막 관측 상태만 다시 조회하며 가져오기를 실행하지 않는다. 표시된 상태는 실시간 최신 보장이 아니다.

## 화면에서 분류하고 연결하기

1. `meenseek` 또는 `개인` 범위를 선택하고 문서 내용·경로·태그를 검색한다. 처음 가져온 문서는 **미분류**이며 `미분류만`으로 모아 볼 수 있다. 목록은 최대 100개이므로 필요하면 검색어로 좁힌다.
2. 목록과 상세에서 Git·Vault 출처를 구분하고 문서를 열어 조회 사본과 출처를 확인한다. Git 자료는 출처 커밋, Vault 자료는 원문 SHA-256과 조회 내용 SHA-256을 보여 준다. `meenseek`은 여러 분야를 선택할 수 있고, 두 범위 모두 문서 태그를 한 줄에 하나씩 최대 10개 저장할 수 있다. 태그 하나는 최대 80자다. `분류 확인 저장`을 누르면 이전 값과 확인한 값이 이력에 남는다.
3. `관련 자료`에서 같은 범위의 문서를 찾아 `연결 추가`를 누른다. 연결은 양쪽 문서에 표시되며 `해제`할 수 있다. 최근 확인·정정 이력에서 변경을 확인한다.

회사의 다섯 분야는 `src/domain.rs`가 소유한다: **전략·포트폴리오, 시장·고객 이해, 제품·서비스 제공, 성장·판매·고객 관계, 경영 기반**. 화면은 이 기준을 받아 사용한다. 개인 자료는 회사 분야를 적용하지 않고 문서 태그로 분류한다. 서로 다른 범위의 문서는 연결하지 않는다. 저장 충돌이 표시되면 자료를 새로고침한 뒤 최신 내용을 확인하고 다시 저장한다.

## 백업과 복원 확인

Git 원문 사본(`source_records`, 화면의 `Git projection`)은 해당 저장소와 커밋으로 다시 만들 수 있다. **기억·기억 묶음·분류·연결·이력은 앱이 소유하는 데이터이므로 원문 재가져오기로 복구할 수 없다.** PostgreSQL 전체 백업을 보관한다. 전체 `pg_dump`에는 `subjects`, `memory_creations`, `memories`, `memory_history`와 중복 생성 방지 기록도 포함된다.

Vault 조회 사본은 현재 파일을 다시 읽어 갱신할 수 있지만, 원문 SHA-256만으로 과거 파일 내용을 복원할 수 없다. 마지막 성공 조회 사본과 앱 기록은 PostgreSQL 백업으로 보관하고, Vault 원본은 Vault 쪽에서 별도로 보존한다.

먼저 `Ctrl-C`로 HTTP 서버와 자동 갱신을 멈추고, 가져오기·`sync-once`·기억 CLI 등 DB 쓰기를 모두 중단한다. PostgreSQL은 실행 상태로 둔다. 다음 Bash 블록은 최종 전체 백업을 custom 형식으로 만들고 **별도의 빈 DB**에 복원한다. 문서·분류·연결·이력과 기억 관련 네 테이블의 건수를 비교하고, 복원 DB에 `init`으로 baseline 검사와 migration을 적용한 뒤 재확인한다. `002-second-brain.sql` 첫 적용 전에는 없는 네 테이블의 건수를 0으로 취급하고, 존재할 때만 실제 조회문을 실행한다. 모두 성공한 뒤 임시 복원 DB만 삭제한다. 운영 DB `ontology`에는 복원을 덮어쓰지 않는다.

```bash
cd /Users/meenseek/Desktop/meenseek-ontology
bash <<'BASH'
set -euo pipefail
set -a
source .env
set +a
umask 077
mkdir -p backups
backup="$PWD/backups/ontology-$(date +%Y%m%d-%H%M%S)-${RANDOM}.dump"
restore_db="ontology_restore_$(date +%Y%m%d%H%M%S)_${RANDOM}"
printf '백업: %s\n임시 복원 DB: %s\n' "$backup" "$restore_db"

docker compose exec -T postgres pg_dump -U ontology -d ontology --format=custom > "$backup"
docker compose exec -T postgres createdb -U ontology --template=template0 "$restore_db"
docker compose exec -T postgres pg_restore -U ontology -d "$restore_db" \
  --exit-on-error --single-transaction < "$backup"

snapshot_counts() {
  docker compose exec -T postgres psql -X -U ontology -d "$1" -At -v ON_ERROR_STOP=1 <<'SQL'
SELECT (SELECT count(*) FROM sources), (SELECT count(*) FROM entities), (SELECT count(*) FROM source_records), (SELECT count(*) FROM entity_areas), (SELECT count(*) FROM entity_topics), (SELECT count(*) FROM related_materials), (SELECT count(*) FROM confirmation_history);
SELECT CASE WHEN to_regclass('public.' || table_name) IS NULL
  THEN format('SELECT %L, 0::bigint;', table_name)
  ELSE format('SELECT %L, count(*) FROM public.%I;', table_name, table_name)
END
FROM (VALUES (1, 'subjects'), (2, 'memory_creations'), (3, 'memories'), (4, 'memory_history')) AS tables(position, table_name)
ORDER BY position
\gexec
SQL
}
original_counts="$(snapshot_counts ontology)"
restored_counts="$(snapshot_counts "$restore_db")"
test "$original_counts" = "$restored_counts"
DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/${restore_db}" \
  cargo run --locked -- init
upgraded_counts="$(snapshot_counts "$restore_db")"
test "$original_counts" = "$upgraded_counts"

docker compose exec -T postgres dropdb -U ontology "$restore_db"
printf '복원 확인 완료. 백업 보관: %s\n' "$backup"
BASH
```

어느 단계든 실패하면 이후 단계는 중단된다. 출력된 백업 경로와 임시 복원 DB 이름으로 원인을 확인한다. 확인 후 정리할 대상은 그 실행에서 만든 `ontology_restore_…` DB뿐이다. `.dump`는 Git에서 제외되며, 백업 파일은 필요한 별도 보관 위치에도 보존한다. 건수 비교는 모든 내용의 동일성을 증명하는 검사는 아니다.

`schema/baseline.sql`은 최초 빈 DB 전용이다. 앱은 초기화 때 적용한 파일의 digest를 저장하고, 이후 다른 digest를 거부한다. 이후 스키마 변경은 `schema/migrations/`에 새 migration을 추가한다. `init`은 재실행 시에도 적용 기록을 검사하며, 이미 적용한 migration의 이름 또는 digest가 달라지면 거부한다. 이미 적용한 baseline과 migration SQL은 수정하거나 기존 DB에 수동으로 다시 적용하지 않는다. 불일치가 발생하면 DB에 적용된 버전의 소스와 백업을 먼저 확인한다.

## 개발 검증

Vault CLI를 먼저 빌드하고, 검증에 사용할 실행 파일을 `TEST_VAULT_BINARY`에 지정한다.

```bash
cd /Users/meenseek/Desktop/llm-context-vault
cargo build --locked -p llm-context-vault
cd /Users/meenseek/Desktop/meenseek-ontology
export TEST_VAULT_BINARY="/Users/meenseek/Desktop/llm-context-vault/target/debug/llm-context-vault"
bash scripts/verify.sh
```

`TEST_VAULT_BINARY`가 없거나 절대 경로의 실행 가능한 파일이 아니면 임시 DB 컨테이너를 만들기 전에 실패한다.

이 스크립트는 새 임시 PostgreSQL 컨테이너와 임시 저장 공간을 만들고 종료 시 제거한다. 앱 DB와 `meenseek-ontology-data` 볼륨은 사용하지 않는다. Rust 형식·컴파일·Clippy·테스트·의존성 보안 점검과 웹 타입 검사·빌드·운영 의존성 보안 점검을 수행한다.

테스트 범위는 Git 커밋 가져오기와 실패·부재 처리, 공식 Vault CLI 읽기 연동, 재가져오기 후 사용자 기록 보존, 검색·분류·연결·이력, 기억의 상태·정정·중복 방지·근거와 회상, 자동 갱신과 실패 격리, scope 격리, 변경 충돌, API 로컬 요청 보호와 입력·응답 제한이다. 웹 빌드는 브라우저 전체 동작 시험을 대신하지 않으며, 위 백업·복원 확인은 `verify.sh`와 별도로 실행한다.
