import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import CareerWorkflow, { StepList, actionText, parseFindings, reviewProposal } from "./CareerWorkflow";
import type { Flow, NativeRef } from "./CareerWorkflow";
const ref:NativeRef={logical_path:"vault/personal/knowledge/reflection.md",state:{state:"live",material_id:"00000000-0000-4000-8000-000000000001",revision:1,content_digest:"a".repeat(64)}};
const flow:Flow={id:"sample",revision:1,application_key:"sample",outcome_key:"rejected",health:"ready",progress:{stage:"review",paused:false}};
test("stages and abnormal state explain the next action without claiming execution",()=>{
  const markup=renderToStaticMarkup(<StepList flow={flow}/>);assert.match(markup,/aria-current="step"/);assert.match(markup,/반영 검토/);
  assert.match(actionText({...flow,health:"source-drift"}),/다시 검토/);
  assert.match(actionText({...flow,health:"native-apply-pending"}),/복구/);
  assert.match(actionText({...flow,progress:{...flow.progress,paused:true}}),/중단/);
  const initial=renderToStaticMarkup(<CareerWorkflow/>);assert.match(initial,/진행을 확인/);assert.doesNotMatch(initial,/적용됐습니다/);
});
test("review drafts cannot promote an unknown cause or omit the revisit condition",()=>{
  const findings=parseFindings('```career-workflow\n{"kind":"reflection","findings":[{"id":"cause","kind":"unknown","statement":"Unknown cause"}]}\n```');
  assert.throws(()=>reviewProposal(ref,findings,{cause:{disposition:"apply",reason:"guess",targets:["criterion"],revisit_when:""}}));
  assert.throws(()=>reviewProposal(ref,findings,{cause:{disposition:"hold",reason:"unknown",targets:[],revisit_when:""}}));
  const proposal=reviewProposal(ref,findings,{cause:{disposition:"hold",reason:"unknown",targets:[],revisit_when:"Employer feedback"}});
  assert.match(proposal,/Employer feedback/);assert.match(proposal,/current|현재 원문/);
  assert.throws(()=>parseFindings('```career-workflow\n{}\n```'));
});
