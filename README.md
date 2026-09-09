# meenseek ontology

Git에 커밋된 문서와 명시적으로 허용한 Vault 문서를 로컬에서 검색하고, 분야·주제와 관련 자료를 직접 확인해 저장하는 앱이다. Git 커밋과 Vault의 공식 `read` 명령으로 가져온 조회 사본을 검색·분류·연결한다.

단일 로컬 사용자용이다. HTTP 서버와 PostgreSQL은 `127.0.0.1`에만 연결하며, 네트워크 배포나 OS 사용자 사이의 인증을 제공하지 않는다. Vault 원본 이전·쓰기·초안 기록·자동 동기화, Workbench·Observatory·Factory 연동, MCP, LLM, 벡터 DB, 자동 작업 큐는 아직 지원하지 않는다.

## 실행과 종료

현재 검증 환경은 Rust **1.98.1**(`rust-toolchain.toml`에 고정), Node **26**, pnpm **11.8.0**, Docker Compose, Git, OpenSSL, 설치된 `cargo-audit`다. Docker가 실행 중이어야 한다.

앱 루트에서 기존 DB와 비밀번호가 일치하는 로컬 `.env`를 사용한다. `ONTOLOGY_DB_PASSWORD`는 영문 대소문자·숫자·`_`·`-`로 된 16~128자다. `.env`는 Git에 넣지 않는다.

### 기존 DB 업그레이드

기존 DB를 사용하는 버전 업그레이드에서는 새 서버를 시작하기 전에 다음 순서를 따른다.

1. `Ctrl-C`로 기존 HTTP 서버를 종료하고 importer 등 모든 DB 쓰기를 중단한다. PostgreSQL은 실행 상태로 둔다.
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

운영 `init`까지 성공한 뒤 아래 앱 시작 명령으로 새 서버를 실행한다. 어느 단계든 실패하면 서버와 importer를 다시 시작하기 전에 원인을 확인한다.

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

동일한 저장소·경로·scope를 다시 가져오면 Git 원문 사본과 출처 확인 정보를 갱신한다. 사용자가 저장한 분류·연결·확인 이력은 유지한다. 고정 커밋에서 파일이 사라진 경우에는 부재를 기록하고 마지막 원문을 보존한다. 읽기 실패 시에도 마지막 성공 자료를 보존하며 화면에 확인 실패를 표시한다.

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

같은 Vault 루트·Vault scope·파일 경로·앱 scope로 다시 가져오면 조회 사본과 출처 확인 정보를 갱신하고, 사용자가 저장한 분류·연결·확인 이력은 유지한다. Vault 읽기가 실패하면 요청 전체의 새 사본을 반영하지 않고, 이미 등록된 요청 자료에는 확인 실패를 표시하며 마지막 성공 사본을 보존한다. **Vault 파일의 부재도 읽기 실패로 처리한다.** Git처럼 고정 커밋에서 확인한 부재 상태를 기록하지 않는다.

Vault가 민감값을 가린 제목·본문으로 조회 사본을 만들기 때문에 원문 SHA-256과 조회 내용 SHA-256은 구분한다. 원문 SHA-256은 가리기 전 파일의 해시이고, 조회 내용 SHA-256은 앱이 저장한 조회 사본의 해시다.

## 화면에서 분류하고 연결하기

1. `meenseek` 또는 `개인` 범위를 선택하고 문서 내용·경로·주제를 검색한다. 처음 가져온 문서는 **미분류**이며 `미분류만`으로 모아 볼 수 있다. 목록은 최대 100개이므로 필요하면 검색어로 좁힌다.
2. 목록과 상세에서 Git·Vault 출처를 구분하고 문서를 열어 조회 사본과 출처를 확인한다. Git 자료는 출처 커밋, Vault 자료는 원문 SHA-256과 조회 내용 SHA-256을 보여 준다. `meenseek`은 여러 분야를 선택할 수 있고, 두 범위 모두 주제를 한 줄에 하나씩 최대 10개 저장할 수 있다. 주제 하나는 최대 80자다. `분류 확인 저장`을 누르면 이전 값과 확인한 값이 이력에 남는다.
3. `관련 자료`에서 같은 범위의 문서를 찾아 `연결 추가`를 누른다. 연결은 양쪽 문서에 표시되며 `해제`할 수 있다. 최근 확인·정정 이력에서 변경을 확인한다.

회사의 다섯 분야는 `src/domain.rs`가 소유한다: **전략·포트폴리오, 시장·고객 이해, 제품·서비스 제공, 성장·판매·고객 관계, 경영 기반**. 화면은 이 기준을 받아 사용한다. 개인 자료는 회사 분야를 적용하지 않고 별도의 주제로 분류한다. 서로 다른 범위의 문서는 연결하지 않는다. 저장 충돌이 표시되면 자료를 새로고침한 뒤 최신 내용을 확인하고 다시 저장한다.

## 백업과 복원 확인

Git 원문 사본(`source_records`, 화면의 `Git projection`)은 해당 저장소와 커밋으로 다시 만들 수 있다. **분류·연결·확인 이력은 앱이 소유하는 데이터이므로 원문 재가져오기로 복구할 수 없다.** PostgreSQL 전체 백업을 보관한다.

Vault 조회 사본은 현재 파일을 다시 읽어 갱신할 수 있지만, 원문 SHA-256만으로 과거 파일 내용을 복원할 수 없다. 마지막 성공 조회 사본과 앱 기록은 PostgreSQL 백업으로 보관하고, Vault 원본은 Vault 쪽에서 별도로 보존한다.

먼저 `Ctrl-C`로 HTTP 서버를 멈추고 가져오기 등 DB 쓰기를 중단한다. PostgreSQL은 실행 상태로 둔다. 다음 Bash 블록은 custom 형식 백업을 만들고 **별도의 빈 DB**에 복원한다. 문서·분류·연결·이력 건수를 비교하고, 복원 DB에 `init`으로 baseline 검사와 migration을 적용한 뒤에도 같은 건수인지 재확인한다. 모두 성공한 뒤 임시 복원 DB만 삭제한다. 운영 DB `ontology`에는 복원을 덮어쓰지 않는다.

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

counts_sql='SELECT (SELECT count(*) FROM sources), (SELECT count(*) FROM entities), (SELECT count(*) FROM source_records), (SELECT count(*) FROM entity_areas), (SELECT count(*) FROM entity_topics), (SELECT count(*) FROM related_materials), (SELECT count(*) FROM confirmation_history);'
original_counts="$(docker compose exec -T postgres psql -X -U ontology -d ontology -At -v ON_ERROR_STOP=1 -c "$counts_sql")"
restored_counts="$(docker compose exec -T postgres psql -X -U ontology -d "$restore_db" -At -v ON_ERROR_STOP=1 -c "$counts_sql")"
test "$original_counts" = "$restored_counts"
DATABASE_URL="postgresql://ontology:${ONTOLOGY_DB_PASSWORD}@127.0.0.1:55432/${restore_db}" \
  cargo run --locked -- init
upgraded_counts="$(docker compose exec -T postgres psql -X -U ontology -d "$restore_db" -At -v ON_ERROR_STOP=1 -c "$counts_sql")"
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

테스트 범위는 Git 커밋 가져오기와 실패·부재 처리, 공식 Vault CLI 읽기 연동, 재가져오기 후 사용자 기록 보존, 검색·분류·연결·이력, scope 격리, 변경 충돌, API 로컬 요청 보호와 입력·응답 제한이다. 웹 빌드는 브라우저 전체 동작 시험을 대신하지 않으며, 위 백업·복원 확인은 `verify.sh`와 별도로 실행한다.
