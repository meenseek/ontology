import { useEffect, useRef, useState } from "react";

export type NativeRef = { logical_path: string; state: { state: "live"; material_id: string; revision: number; content_digest: string } };
export type Stage = "reflection" | "review" | "criteria-apply" | "output-verification" | "complete";
export type Flow = { id: string; revision: number; application_key: string; outcome_key: string; health?: string; progress: { stage: Stage; paused: boolean; criteria?: NativeRef[]; reflection?: NativeRef | null; review?: NativeRef | null; verification?: NativeRef | null; apply?: { run_id: string; apply_id: string | null } | null } };
type Finding = { id: string; kind: string; statement: string };
type Choice = { disposition: string; reason: string; targets: string[]; revisit_when: string };
type Material = { content: string; metadata: { revision: number; content_digest: string } };
export const stages: [Stage, string][] = [["reflection", "복기"], ["review", "반영 검토"], ["criteria-apply", "작성 기준 갱신"], ["output-verification", "새 문서 확인"], ["complete", "완료"]];
const next: Record<Stage, string> = { reflection: "당시 제출 자료와 공고를 확인해 복기를 이어갑니다.", review: "항목별로 반영·보류·기각·기존 기준 해결 여부를 검토합니다.", "criteria-apply": "검토된 변경을 적용하고 실제 작성 기준이 바뀌었는지 확인합니다.", "output-verification": "최신 기준으로 새 문서를 작성하고 반영된 내용을 독립 검토합니다.", complete: "이번 흐름의 검토와 새 문서 확인을 마쳤습니다." };
export function actionText(flow: Flow): string {
  if (flow.health === "source-drift") return "원문이 바뀌었습니다. 최신 입력으로 복기를 다시 검토해야 합니다.";
  if (flow.health === "native-apply-pending") return "기존 원문 적용의 복구가 필요합니다. 기존 실행 결과를 먼저 확인해야 합니다.";
  if (flow.progress.paused) return "중단된 단계입니다. 재개하면 기존 진행을 이어갑니다.";
  return next[flow.progress.stage];
}
export function StepList({ flow }: { flow: Flow }) {
  const current = stages.findIndex(([stage]) => stage === flow.progress.stage);
  return <ol className="career-steps" aria-label="지원 결과 처리 단계">{stages.map(([stage, label], index) => <li key={stage} aria-current={stage === flow.progress.stage ? "step" : undefined} className={index < current ? "done" : stage === flow.progress.stage ? "current" : ""}><span>{index < current ? "✓" : index + 1}</span>{stage === "criteria-apply" && ["output-verification", "complete"].includes(flow.progress.stage) && !flow.progress.apply ? "기준 유지 확인" : label}</li>)}</ol>;
}
export function parseFindings(content: string): Finding[] {
  const found = content.split("```career-workflow\n");
  if (found.length !== 2) throw new Error("복기 기록 형식을 확인할 수 없습니다.");
  const record = JSON.parse(found[1].split("\n```")[0]);
  if (record.kind !== "reflection" || !Array.isArray(record.findings)) throw new Error("복기 기록 형식을 확인할 수 없습니다.");
  return record.findings;
}
export function reviewProposal(reflection: NativeRef, findings: Finding[], choices: Record<string, Choice>): string {
  const decisions = findings.map(finding => {
    const choice = choices[finding.id];
    if (!choice || !choice.reason.trim() || choice.disposition === "hold" && !choice.revisit_when.trim() || ["apply", "already-covered"].includes(choice.disposition) && !choice.targets.length || ["hypothesis", "unknown"].includes(finding.kind) && choice.disposition === "apply") throw new Error("모든 항목의 판단·이유와 필요한 기준 또는 재검토 조건을 입력하세요.");
    return { finding_id: finding.id, disposition: choice.disposition, reason: choice.reason.trim(), targets: choice.targets, revisit_when: choice.disposition === "hold" ? choice.revisit_when.trim() : null };
  });
  return "이 지원 결과의 검토 의견입니다. 현재 원문과 대조해 검토 기록으로 반영하고, 채택한 변경만 작성 기준에 적용한 뒤 새 문서에서 확인해 주세요.\n\n```career-workflow\n" + JSON.stringify({ kind: "review", reflection, decisions }, null, 2) + "\n```";
}
async function request<T>(url: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try { response = await fetch(url, { ...init, credentials: "same-origin" }); } catch { throw new Error("연결할 수 없습니다. 앱 실행 상태를 확인한 뒤 다시 시도하세요."); }
  const result = await response.json();
  if (!response.ok) throw new Error(response.status === 409 ? "진행 또는 원문이 바뀌었습니다. 새로 확인한 뒤 이어가세요." : response.status === 403 ? "연결이 만료됐습니다. 페이지를 다시 열어주세요." : response.status === 413 ? "입력 한도를 넘었습니다." : "진행을 불러오지 못했습니다. 요청과 앱 상태를 확인하세요.");
  return result;
}
function selection(ref: NativeRef) {
  const parts = ref.logical_path.slice(6).split("/");
  const scope = parts[0] === "work" ? parts.splice(0, 2).join("/") : parts.shift()!;
  return { scope, path: parts.join("/") };
}
function RefLink({ reference }: { reference: NativeRef }) {
  const { scope, path } = selection(reference);
  return <a href={`/?scope=personal&q=${encodeURIComponent(path)}`} title={`${scope} · ${path}`}>{path}<small> · 기준 {reference.state.revision}</small></a>;
}
export default function CareerWorkflow() {
  const [flows, setFlows] = useState<Flow[]>([]), [flow, setFlow] = useState<Flow | null>(null);
  const [session, setSession] = useState<{ csrf: string } | null>(null), [storeId, setStoreId] = useState("");
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [error, setError] = useState(""), [notice, setNotice] = useState("");
  const [reflectionWarning, setReflectionWarning] = useState("");
  const [findings, setFindings] = useState<Finding[]>([]), [choices, setChoices] = useState<Record<string, Choice>>({}), [proposal, setProposal] = useState("");
  const generation = useRef(0), selected = useRef(""), draftIdentity = useRef("");
  async function load(id = selected.current) {
    const token = ++generation.current; setLoading(true); setError("");
    try {
      const authenticated = await request<{ csrf: string }>("/api/session");
      const list = await request<{ store_id: string; workflows: Flow[] }>("/api/career");
      const wanted = id || list.workflows[0]?.id;
      const detail = wanted ? await request<Flow>(`/api/career/${wanted}?store_id=${encodeURIComponent(list.store_id)}`) : null;
      let readFindings: Finding[] = [], warning = "";
      if (detail?.progress.reflection) {
        if (detail.health !== "ready") warning = "현재 원문을 먼저 재확인한 뒤 복기 내용을 검토합니다.";
        else {
          try {
            const ref = detail.progress.reflection, { scope, path } = selection(ref);
            const material = await request<Material>(`/api/context/read?${new URLSearchParams({ scope, path })}`);
            if (material.metadata.revision !== ref.state.revision || material.metadata.content_digest !== ref.state.content_digest) throw new Error("복기 원문이 바뀌었습니다. 기존 판단을 다시 확인하세요.");
            readFindings = parseFindings(material.content);
          } catch (e) { warning = (e as Error).message; }
        }
      }
      if (token !== generation.current) return;
      selected.current = detail?.id || ""; setSession(authenticated); setStoreId(list.store_id); setFlows(list.workflows); setFlow(detail); setFindings(readFindings); setReflectionWarning(warning);
      const identity = JSON.stringify([detail?.id, detail?.progress.reflection, detail?.progress.criteria]);
      if (draftIdentity.current !== identity) { setChoices({}); setProposal(""); draftIdentity.current = identity; }
    } catch (e) { if (token === generation.current) { setFlow(null); setError((e as Error).message); } }
    finally { if (token === generation.current) setLoading(false); }
  }
  useEffect(() => { void load(); return () => { generation.current++; }; }, []);
  async function mutate(op: "pause" | "resume") {
    if (!flow || !session || busy) return;
    setBusy(true); setError(""); setNotice("");
    try { await request("/api/career", { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": session.csrf }, body: JSON.stringify({ op, store_id: storeId, id: flow.id, expected_revision: flow.revision }) }); await load(flow.id); }
    catch (e) { setError((e as Error).message); }
    finally { setBusy(false); }
  }
  async function copy(text: string) {
    setProposal(text);
    try { await navigator.clipboard.writeText(text); setNotice("요청을 복사했습니다. 현재 대화에 전달하면 이어갑니다."); }
    catch { setNotice("아래 요청을 선택해 현재 대화에 전달하세요."); }
  }
  function choose(id: string, update: Partial<Choice>) { setChoices(current => ({ ...current, [id]: { ...(current[id] || { disposition: "hold", reason: "", targets: [], revisit_when: "" }), ...update } })); }
  return <div className="career-page">
    <header className="career-header"><a className="wordmark" href="/">개인 온톨로지</a><span>지원 작업공간</span><button disabled={loading || busy} onClick={() => { setNotice(""); void load(); }}>새로 확인</button></header>
    <main className="career-main"><div className="career-title"><div><p className="career-eyebrow">지원 결과에서 다음 문서까지</p><h1>복기하고, 검토하고, 반영 확인하기</h1></div></div>
      {error && <div role="alert" className="career-alert">{error}<button disabled={loading || busy} onClick={() => void load()}>다시 확인</button></div>}
      {notice && <p role="status" className="career-notice">{notice}</p>}
      {loading ? <p role="status">진행을 확인하고 있습니다…</p> : !flows.length && !error ? <section className="career-empty"><h2>아직 연결된 지원 결과가 없습니다</h2><p>현재 대화에서 회사·직무와 지원 결과를 알려주세요. 실제 제출 자료를 확인한 뒤 복기부터 연결합니다.</p><p>탈락 사유가 공개되지 않았으면 문서에서 확인한 점과 추정을 구분합니다.</p></section> : <div className="career-layout">
        <nav className="career-cases" aria-label="지원 결과 목록"><h2>최근 지원 결과</h2>{flows.map(item => <button key={item.id} disabled={busy || loading} aria-current={item.id === flow?.id ? "true" : undefined} onClick={() => { if (item.id === selected.current || !Object.keys(choices).length || confirm("검토 의견 초안을 닫고 다른 지원 결과를 확인할까요?")) void load(item.id); }}><strong>{item.application_key}</strong><small>{stages.find(([s]) => s === item.progress.stage)?.[1]}{item.progress.paused ? " · 중단" : ""}</small></button>)}<small>최근 100건까지 표시합니다.</small></nav>
        {flow && <section className="career-detail" aria-label="선택한 지원 결과"><div className="career-case-heading"><h2>{flow.application_key}</h2>{flow.progress.stage !== "complete" && <button disabled={busy || loading || flow.health === "native-apply-pending" && flow.progress.paused} onClick={() => void mutate(flow.progress.paused ? "resume" : "pause")}>{flow.progress.paused ? "재개" : "중단"}</button>}</div><StepList flow={flow} /><p className="career-next">{actionText(flow)}</p>{reflectionWarning && <p role="status" className="career-notice">{reflectionWarning}</p>}
          {flow.progress.stage !== "complete" && <button disabled={busy} onClick={() => void copy(`지원 결과 흐름 ${flow.id}를 현재 원문과 기존 적용 이력부터 확인해 이어가 주세요. Store identity: ${storeId}. ${actionText(flow)} 새 실행을 중복 시작하지 말고 기존 단계에서 이어가세요.`)}>이어갈 요청 복사</button>}
          {!!flow.progress.criteria?.length && <section><h3>이번에 검토하는 작성 기준</h3><ul>{flow.progress.criteria.map(ref => <li key={ref.logical_path}><RefLink reference={ref} /></li>)}</ul></section>}
          <section><h3>연결된 기록</h3><ul>{(["reflection", "review", "verification"] as const).flatMap(kind => { const ref = flow.progress[kind]; return ref ? [<li key={kind}><span>{kind === "reflection" ? "복기" : kind === "review" ? "반영 검토" : "새 문서 확인"} · </span><RefLink reference={ref} /></li>] : []; })}</ul>{flow.progress.apply?.apply_id && <p className="hint">작성 기준의 실제 원문 적용이 확인됐습니다.</p>}</section>
          {flow.progress.stage === "review" && findings.length > 0 && <form className="career-review" onSubmit={event => { event.preventDefault(); try { void copy(reviewProposal(flow.progress.reflection!, findings, choices)); setError(""); } catch (e) { setError((e as Error).message); } }}><h3>복기 항목을 함께 검토</h3><p>의견을 현재 대화에 전달하면 원문과 대조해 반영합니다. 이 화면에서 고른 의견은 전달 전 초안입니다.</p>{findings.map(finding => <fieldset key={finding.id}><legend>{finding.statement}</legend><small>{({ "confirmed-fact": "확인된 사실", "artifact-observation": "문서에서 확인", hypothesis: "가설", unknown: "미확인" } as Record<string, string>)[finding.kind]}</small><label>판단<select disabled={busy} value={choices[finding.id]?.disposition || "hold"} onChange={event => choose(finding.id, { disposition: event.target.value })}><option value="hold">보류</option><option value="apply" disabled={["hypothesis", "unknown"].includes(finding.kind)}>반영</option><option value="reject">기각</option><option value="already-covered">기존 기준으로 해결</option></select></label><label>이유<textarea required maxLength={4096} disabled={busy} value={choices[finding.id]?.reason || ""} onChange={event => choose(finding.id, { reason: event.target.value })} /></label>{["apply", "already-covered"].includes(choices[finding.id]?.disposition) ? <label>연결할 기준<select required value={choices[finding.id]?.targets[0] || ""} onChange={event => choose(finding.id, { targets: event.target.value ? [event.target.value] : [] })}><option value="">기준 선택</option>{flow.progress.criteria?.map(ref => <option key={ref.logical_path} value={ref.logical_path}>{selection(ref).path}</option>)}</select></label> : (choices[finding.id]?.disposition || "hold") === "hold" && <label>다시 검토할 조건<input required maxLength={4096} value={choices[finding.id]?.revisit_when || ""} onChange={event => choose(finding.id, { revisit_when: event.target.value })} /></label>}</fieldset>)}<button type="submit" disabled={busy || flow.health !== "ready" || flow.progress.paused}>검토 의견 복사</button></form>}
        </section>}
      </div>}
      {proposal && <section className="career-proposal"><label>현재 대화에 전달할 요청<textarea readOnly value={proposal} onFocus={event => event.currentTarget.select()} /></label></section>}
    </main>
  </div>;
}
