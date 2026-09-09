import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import Memory from "./Memory";
type Scope = "meenseek" | "personal";
type SourceKind = "git" | "vault";
type Area = { id: string; label: string };
type RecordItem = {
  id: string;
  path: string;
  kind: SourceKind;
  areas: string[];
  topics: string[];
  excerpt: string;
  status: string;
  present: boolean;
};
type Listing = {
  items: RecordItem[];
  total: number;
  limit: number;
  areas: { area: string; count: number }[];
};
type Detail = {
  scope: Scope;
  id: string;
  revision: number;
  areas: string[];
  topics: string[];
  source: {
    kind: SourceKind;
    repository: string;
    path: string;
    status: string;
    verified_revision: string | null;
    last_attempt_at: string;
    last_success_at: string | null;
  };
  projection: {
    content: string | null;
    content_digest: string | null;
    source_revision: string | null;
    present: boolean;
    absence_revision: string | null;
    observed_at: string;
  };
  related: { id: string; path: string }[];
  history: {
    id: number;
    kind: string;
    revision: number;
    previous: unknown;
    confirmed: unknown;
    confirmed_at: string;
  }[];
};
const empty: Listing = { items: [], total: 0, limit: 100, areas: [] };
class ApiError extends Error {
  constructor(public status: number) {
    super(
      status === 409
        ? "다른 변경이 먼저 저장되었습니다. 새로고침한 뒤 다시 확인해 주세요."
        : status === 403
          ? "세션을 확인할 수 없습니다. 페이지를 새로고침해 주세요."
          : "요청을 완료하지 못했습니다. 입력과 서버 상태를 확인해 주세요.",
    );
  }
}
async function request<T>(url: string, options?: RequestInit): Promise<T> {
  const response = await fetch(url, { credentials: "same-origin", ...options });
  if (!response.ok) throw new ApiError(response.status);
  return response.json() as Promise<T>;
}
const date = (value: string | null) =>
  value ? new Date(value).toLocaleString("ko-KR") : "아직 확인되지 않음";
const message = (error: unknown) =>
  error instanceof Error ? error.message : "요청을 완료하지 못했습니다.";
export default function App() {
  const [view, setView] = useState<"memories" | "sources">("memories");
  const [scope, setScope] = useState<Scope>("meenseek");
  const [session, setSession] = useState<{
    csrf: string;
    areas: Area[];
  } | null>(null);
  const [input, setInput] = useState("");
  const [query, setQuery] = useState("");
  const [unclassified, setUnclassified] = useState(false);
  const [areaFilter, setAreaFilter] = useState("");
  const [storedListing, setListing] = useState<Listing & { scope?: Scope }>(
    empty,
  );
  const listing = storedListing.scope === scope ? storedListing : empty;
  const [selected, setSelected] = useState<string | null>(null);
  const [storedDetail, setDetail] = useState<Detail | null>(null);
  const detail =
    storedDetail?.scope === scope && storedDetail.id === selected
      ? storedDetail
      : null;
  const context = useRef({ scope, selected });
  context.current = { scope, selected };
  const listRequest = useRef(0);
  const detailRequest = useRef(0);
  const choose = (id: string) => {
    if (context.current.selected === id) return;
    context.current.selected = id;
    detailRequest.current += 1;
    setDetail(null);
    setSelected(id);
  };
  const [areas, setAreas] = useState<string[]>([]);
  const [topics, setTopics] = useState("");
  const [relatedQuery, setRelatedQuery] = useState("");
  const [candidates, setCandidates] = useState<RecordItem[]>([]);
  const [target, setTarget] = useState("");
  const [loading, setLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    request<{ csrf: string; areas: Area[] }>("/api/session", {
      signal: controller.signal,
    })
      .then(setSession)
      .catch((e) => {
        if (!controller.signal.aborted) {
          setError(message(e));
          setLoading(false);
        }
      });
    return () => controller.abort();
  }, []);
  useEffect(() => {
    if (!session || view !== "sources") return;
    const controller = new AbortController();
    const sequence = ++listRequest.current;
    setLoading(true);
    setListing(empty);
    request<Listing>(
      `/api/records?scope=${scope}&q=${encodeURIComponent(query)}&unclassified=${unclassified}${areaFilter ? `&area=${encodeURIComponent(areaFilter)}` : ""}`,
      { signal: controller.signal },
    )
      .then((value) => {
        if (
          !controller.signal.aborted &&
          context.current.scope === scope &&
          listRequest.current === sequence
        )
          setListing({ ...value, scope });
      })
      .catch((e) => {
        if (!controller.signal.aborted) setError(message(e));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [session, scope, query, unclassified, areaFilter, refresh, view]);
  useEffect(() => {
    if (!session || !selected) {
      setDetail(null);
      setDetailLoading(false);
      return;
    }
    const controller = new AbortController();
    const sequence = ++detailRequest.current;
    setDetail(null);
    setDetailLoading(true);
    setCandidates([]);
    setTarget("");
    request<Detail>(`/api/records/${selected}?scope=${scope}`, {
      signal: controller.signal,
    })
      .then((value) => {
        if (
          !controller.signal.aborted &&
          context.current.scope === scope &&
          context.current.selected === selected &&
          detailRequest.current === sequence
        ) {
          setDetail({ ...value, scope });
          setAreas(value.areas);
          setTopics(value.topics.join("\n"));
        }
      })
      .catch((e) => {
        if (!controller.signal.aborted) setError(message(e));
      })
      .finally(() => {
        if (!controller.signal.aborted) setDetailLoading(false);
      });
    return () => controller.abort();
  }, [session, scope, selected, refresh]);
  const switchScope = (next: Scope) => {
    context.current = { scope: next, selected: null };
    listRequest.current += 1;
    detailRequest.current += 1;
    setAreaFilter("");
    setScope(next);
    setSelected(null);
    setDetail(null);
    setListing(empty);
    setInput("");
    setQuery("");
    setUnclassified(false);
    setError("");
    setNotice("");
    setCandidates([]);
  };
  const search = (event: FormEvent) => {
    event.preventDefault();
    setQuery(input);
    setError("");
    setRefresh((v) => v + 1);
  };
  const label = (id: string) =>
    session?.areas.find((area) => area.id === id)?.label ?? id;
  async function save(kind: "classification" | "links", body: object) {
    if (!detail || !session) return;
    setSaving(true);
    setError("");
    setNotice("");
    try {
      await request(`/api/records/${detail.id}/${kind}?scope=${scope}`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-csrf-token": session.csrf,
        },
        body: JSON.stringify({ revision: detail.revision, ...body }),
      });
      setNotice("확인 기록을 저장했습니다.");
      setRefresh((v) => v + 1);
    } catch (e) {
      setError(message(e));
    } finally {
      setSaving(false);
    }
  }
  async function findRelated(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    setError("");
    try {
      const result = await request<Listing>(
        `/api/records?scope=${scope}&q=${encodeURIComponent(relatedQuery)}`,
      );
      setCandidates(result.items.filter((item) => item.id !== selected));
      setTarget("");
    } catch (e) {
      setError(message(e));
    } finally {
      setSaving(false);
    }
  }
  return (
    <div className="app">
      <header>
        <a className="wordmark" href="/">
          meenseek<span> / ontology</span>
        </a>
        <span className="local">
          <i />
          로컬 자료 연결
        </span>
      </header>
      <main>
        <section className="intro">
          <div>
            <p className="eyebrow">KNOWLEDGE, IN CONTEXT</p>
            <h1>
              기억과 자료를 찾고,
              <br />
              맥락을 연결하세요.
            </h1>
            <p className="lead">
              기억을 보관하고, 원문 자료를 정리하며,
              <br className="desktop" /> 필요한 맥락과 관련 자료를 찾습니다.
            </p>
          </div>
          <div className="scope-panel">
            <span className="field-label">탐색 범위</span>
            <div className="scope-switch" aria-label="탐색 범위">
              <button
                className={scope === "meenseek" ? "active" : ""}
                disabled={saving}
                onClick={() => switchScope("meenseek")}
              >
                meenseek
              </button>
              <button
                className={scope === "personal" ? "active" : ""}
                disabled={saving}
                onClick={() => switchScope("personal")}
              >
                개인
              </button>
            </div>
            <p>
              {scope === "meenseek"
                ? "회사의 기억과 원문 자료를 살펴봅니다."
                : "개인의 기억과 원문 자료를 별도로 살펴봅니다."}
            </p>
          </div>
        </section>
        <div aria-live="polite">
          {error && (
            <div className="error" role="alert">
              {error}
              <button
                onClick={() => {
                  setError("");
                  setRefresh((v) => v + 1);
                }}
              >
                자료 새로고침
              </button>
            </div>
          )}
          {notice && <div className="notice">{notice}</div>}
        </div>
        <nav className="view-tabs" aria-label="기억과 자료">
          <button disabled={saving} className={view === "memories" ? "primary" : ""} onClick={() => setView("memories")}>기억</button>
          <button disabled={saving} className={view === "sources" ? "primary" : ""} onClick={() => setView("sources")}>원문 자료</button>
        </nav>
        {view === "memories" && session ? <Memory key={scope} scope={scope} csrf={session.csrf} request={request} onBusy={setSaving} /> : null}
        {view === "sources" && <section className="workspace">
          <aside className="library">
            <form className="search" onSubmit={search}>
              <label htmlFor="search">자료 검색</label>
              <div>
                <input
                  id="search"
                  value={input}
                  maxLength={120}
                  onChange={(e) => setInput(e.target.value)}
                  placeholder="문서 내용, 경로, 문서 태그 검색"
                />
                <button disabled={!session || saving} type="submit">
                  검색
                </button>
              </div>
            </form>
            {scope === "meenseek" && (
              <label className="facet-filter">
                분야로 탐색
                <select
                  aria-label="분야로 탐색"
                  value={areaFilter}
                  onChange={(e) => setAreaFilter(e.target.value)}
                >
                  <option value="">전체 분야</option>
                  {session?.areas.map((area) => (
                    <option key={area.id} value={area.id}>
                      {area.label}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <div className="list-meta">
              <span>
                {loading ? "불러오는 중…" : `${listing.total}개 자료`}
              </span>
              <label>
                <input
                  type="checkbox"
                  checked={unclassified}
                  onChange={(e) => setUnclassified(e.target.checked)}
                />{" "}
                미분류만
              </label>
            </div>
            <div className="record-list" aria-busy={loading}>
              {!loading && listing.items.length === 0 && (
                <div className="empty">
                  <b>표시할 자료가 없습니다.</b>
                  <p>
                    {query || unclassified
                      ? "검색어나 미분류 조건을 바꿔보세요."
                      : "명시적으로 가져온 Git·Vault 문서가 이곳에 나타납니다."}
                  </p>
                </div>
              )}
              {listing.items.map((item) => (
                <button
                  key={item.id}
                  className={`record ${selected === item.id ? "selected" : ""}`}
                  disabled={saving}
                  onClick={() => {
                    choose(item.id);
                    setError("");
                    setNotice("");
                  }}
                >
                  <span className="record-kind">
                    {item.kind === "vault" ? "VAULT DOCUMENT" : "GIT DOCUMENT"}{" "}
                    <span>
                      {item.status === "failed"
                        ? "확인 실패"
                        : item.present
                          ? "출처 확인"
                          : "부재 확인"}
                    </span>
                  </span>
                  <strong>{item.path}</strong>
                  <p>{item.excerpt}</p>
                  <div className="tags">
                    {item.areas.map((area) => (
                      <span key={area}>{label(area)}</span>
                    ))}
                    {item.topics.map((topic) => (
                      <span key={topic}>{topic}</span>
                    ))}
                    {item.areas.length + item.topics.length === 0 && (
                      <span>미분류</span>
                    )}
                  </div>
                </button>
              ))}
            </div>
            {listing.total > listing.limit && (
              <p className="hint">
                앞의 {listing.limit}개만 표시합니다. 검색어로 범위를 좁혀주세요.
              </p>
            )}
          </aside>
          <article className="detail" aria-busy={detailLoading}>
            {detailLoading ? (
              <div className="empty">자료를 불러오고 있습니다…</div>
            ) : !detail ? (
              <div className="welcome">
                <div className="connection-symbol">↗</div>
                <p className="eyebrow">START WITH A SOURCE</p>
                <h2>자료 하나에서 시작하세요.</h2>
                <p>
                  목록에서 자료를 선택하면 원문과 출처,
                  <br />
                  분류와 관련 자료를 함께 볼 수 있습니다.
                </p>
                <div className="boundary-note">
                  Git 문서와 지정한 Vault 문서를 가져옵니다.
                  <br />
                  Vault 쓰기와 다른 도구 연결은 아직 지원하지 않습니다.
                </div>
              </div>
            ) : (
              <>
                <div className="detail-heading">
                  <span className="eyebrow">SOURCE DOCUMENT</span>
                  <button
                    className="quiet"
                    disabled={saving}
                    onClick={() => {
                      setError("");
                      setRefresh((v) => v + 1);
                    }}
                  >
                    새로고침
                  </button>
                  <h2>{detail.source.path}</h2>
                </div>
                {detail.source.status === "failed" && (
                  <p className="error">
                    최근 출처 확인에 실패했습니다. 아래 내용은 마지막으로 성공한
                    기록입니다.
                  </p>
                )}
                {!detail.projection.present && (
                  <p className="warning">
                    등록한 경로의 부재를 확인했습니다. 마지막 원문과 사용자의
                    확인 기록은 보존되어 있습니다.
                  </p>
                )}
                <section className="section">
                  <h3>
                    분야와 문서 태그 <span>사용자 확인</span>
                  </h3>
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      void save("classification", {
                        areas,
                        topics: topics
                          .split("\n")
                          .map((v) => v.trim())
                          .filter(Boolean),
                      });
                    }}
                  >
                    <fieldset disabled={saving}>
                      <legend>
                        {scope === "meenseek"
                          ? "분야 · 여러 개 선택 가능"
                          : "개인 자료의 문서 태그"}
                      </legend>
                      {scope === "meenseek" && (
                        <div className="area-options">
                          {session?.areas.map((area) => (
                            <label key={area.id}>
                              <input
                                type="checkbox"
                                checked={areas.includes(area.id)}
                                onChange={(e) =>
                                  setAreas((current) =>
                                    e.target.checked
                                      ? [...current, area.id]
                                      : current.filter((id) => id !== area.id),
                                  )
                                }
                              />
                              {area.label}
                            </label>
                          ))}
                        </div>
                      )}
                      <label className="topic-label" htmlFor="topics">
                        문서 태그 · 한 줄에 하나, 최대 10개
                      </label>
                      <textarea
                        id="topics"
                        value={topics}
                        onChange={(e) => setTopics(e.target.value)}
                        rows={2}
                        maxLength={810}
                        placeholder="아직 분류하지 않았다면 비워두세요."
                      />
                      <button className="primary" type="submit">
                        {saving ? "저장 중…" : "분류 확인 저장"}
                      </button>
                    </fieldset>
                  </form>
                </section>
                <section className="section">
                  <h3>
                    관련 자료 <span>{detail.related.length}개 연결</span>
                  </h3>
                  <ul className="related">
                    {detail.related.map((item) => (
                      <li key={item.id}>
                        <button
                          disabled={saving}
                          onClick={() => choose(item.id)}
                        >
                          {item.path} ↗
                        </button>
                        <button
                          className="quiet"
                          disabled={saving}
                          aria-label={`${item.path} 연결 해제`}
                          onClick={() =>
                            void save("links", {
                              target_id: item.id,
                              remove: true,
                            })
                          }
                        >
                          해제
                        </button>
                      </li>
                    ))}
                  </ul>
                  {detail.related.length === 0 && (
                    <p className="hint">아직 연결한 자료가 없습니다.</p>
                  )}
                  <form className="related-search" onSubmit={findRelated}>
                    <input
                      aria-label="관련 자료 검색"
                      maxLength={120}
                      value={relatedQuery}
                      onChange={(e) => setRelatedQuery(e.target.value)}
                      placeholder="같은 범위에서 연결할 자료 검색"
                    />
                    <button disabled={saving}>찾기</button>
                  </form>
                  <div className="link-controls">
                    <select
                      aria-label="연결할 자료"
                      value={target}
                      onChange={(e) => setTarget(e.target.value)}
                    >
                      <option value="">연결할 자료 선택</option>
                      {candidates.map((item) => (
                        <option key={item.id} value={item.id}>
                          {item.path}
                        </option>
                      ))}
                    </select>
                    <button
                      disabled={!target || saving}
                      onClick={() =>
                        void save("links", { target_id: target, remove: false })
                      }
                    >
                      연결 추가
                    </button>
                  </div>
                </section>
                <section className="section">
                  <h3>
                    원문{" "}
                    <span>
                      {detail.source.kind === "vault"
                        ? "Vault에서 읽은 내용"
                        : "Git에서 읽은 내용"}
                    </span>
                  </h3>
                  <pre className="source-text">
                    {detail.projection.content ??
                      "이 경로에서 성공적으로 읽은 원문이 없습니다."}
                  </pre>
                </section>
                <section className="section provenance">
                  <h3>출처와 확인 시점</h3>
                  <dl>
                    <dt>출처</dt>
                    <dd>{detail.source.kind === "vault" ? "Vault" : "Git"}</dd>
                    <dt>{detail.source.kind === "vault" ? "Vault 경로" : "저장소"}</dt>
                    <dd>{detail.source.repository}</dd>
                    <dt>{detail.source.kind === "vault" ? "원문 SHA-256" : "원문 커밋"}</dt>
                    <dd>{detail.projection.source_revision ?? "없음"}</dd>
                    <dt>{detail.source.kind === "vault" ? "확인한 원문 SHA-256" : "확인 커밋"}</dt>
                    <dd>{detail.source.verified_revision ?? "없음"}</dd>
                    <dt>조회 내용 SHA-256</dt>
                    <dd>{detail.projection.content_digest ?? "없음"}</dd>
                    <dt>원문 관측</dt>
                    <dd>{date(detail.projection.observed_at)}</dd>
                    <dt>마지막 성공</dt>
                    <dd>{date(detail.source.last_success_at)}</dd>
                    <dt>최근 확인 시도</dt>
                    <dd>{date(detail.source.last_attempt_at)}</dd>
                  </dl>
                </section>
                <details className="section">
                  <summary>
                    최근 확인·정정 이력 ({detail.history.length}개)
                  </summary>
                  {detail.history.map((history) => (
                    <div className="history" key={history.id}>
                      <b>
                        {history.kind === "classification"
                          ? "분류 확인"
                          : history.kind === "link-add"
                            ? "관련 자료 추가"
                            : "관련 자료 해제"}{" "}
                        · r{history.revision}
                      </b>
                      <time>{date(history.confirmed_at)}</time>
                      <pre>
                        {JSON.stringify(
                          { 이전: history.previous, 확인: history.confirmed },
                          null,
                          2,
                        )}
                      </pre>
                    </div>
                  ))}
                </details>
              </>
            )}
          </article>
        </section>}
        <footer>
          <span>meenseek ontology</span>
          <span>
            출처의 원문과 사용자의 분류·연결 기록을 구분해 보존합니다.
          </span>
        </footer>
      </main>
    </div>
  );
}
