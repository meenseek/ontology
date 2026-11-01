import {test,expect,type Page} from "playwright/test";
import {createMapFixture} from "../src/map-fixture";
import {installGraphProbe,settledFramesMatch} from "./graph-probe";

declare global { interface Window { __mapGpu: { programs:number; deleted:number; shadersDisposed:number; planesDisposed:number; duplicateLines:number; duplicateArrows:number } } }
function observeGpu() {
 const totals=window.__mapGpu={programs:0,deleted:0,shadersDisposed:0,planesDisposed:0,duplicateLines:0,duplicateArrows:0};
 for(const [method,key] of [["createProgram","programs"],["deleteProgram","deleted"]] as const){
  const original=WebGL2RenderingContext.prototype[method];
  (WebGL2RenderingContext.prototype as any)[method]=function(...args:unknown[]){totals[key]++;
  return (original as any).apply(this,args);};
 }
 const watched=new WeakSet<object>();
 (window as any).__THREE_DEVTOOLS__.addEventListener("observe",(event:any)=>{
  const scene=event.detail;if(!scene.isScene)return;
  const before=scene.onBeforeRender;
  scene.onBeforeRender=function(...args:unknown[]){
   totals.duplicateLines=0;totals.duplicateArrows=0;
   scene.traverse((object:any)=>{
    const material=object.material,geometry=object.geometry,edge=object.__data;
    const endpoint=(value:any)=>typeof value==="object"?value?.id:value;
    if(edge&&endpoint(edge.source)==="preview-1-0"&&endpoint(edge.target)==="preview-1-1"&&edge.kind==="reference"&&edge.current){
     if(object.__graphObjType==="link")totals.duplicateLines++;
     if(object.__linkThreeObjType==="arrow")totals.duplicateArrows++;
    }
    for(const [resource,key] of [[material?.type==="ShaderMaterial"?material:null,"shadersDisposed"],[geometry?.type==="PlaneGeometry"?geometry:null,"planesDisposed"]] as const){
     if(resource&&!watched.has(resource)){watched.add(resource);resource.addEventListener("dispose",()=>{totals[key]++;});}
    }
   });before.apply(this,args);
  };
 });
}
async function settled(page:Page){
 await page.evaluate(()=>document.fonts.ready);
  let previous="",serial=-1,stable=0;
 await expect.poll(async()=>{const frame=await page.evaluate(()=>window.__graphQA?.frame());if(!frame?.stars.length||frame.serial===serial)return stable;serial=frame.serial;
  const current=JSON.stringify({...frame,serial:0});stable=settledFramesMatch(previous,current)?stable+1:0;previous=current;
  return stable;},{timeout:30_000,intervals:[100]}).toBeGreaterThanOrEqual(5);
}
async function expand(page:Page){const button=page.locator('.graph-group-target[data-node-id="preview-purpose-1"]');
  await button.focus();
  await button.press("Enter");
  await expect(page.getByRole("button",{name:"묶음 접기",exact:true})).toBeVisible();
  await settled(page);
  await expect.poll(()=>page.evaluate(()=>({lines:window.__mapGpu.duplicateLines,arrows:window.__mapGpu.duplicateArrows}))).toEqual({lines:2,arrows:2});}
async function choose(page:Page){
 const point=await page.evaluate(()=>{const canvas=document.querySelector(".graph-viewport canvas[data-engine]")!.getBoundingClientRect(),star=window.__graphQA.frame()!.stars.find(s=>s.id==="preview-1-0")!;
  return{x:canvas.left+star.x,y:canvas.top+star.y};});
 await page.mouse.click(point.x,point.y);
  await expect(page.getByRole("dialog",{name:"선택한 자료 상세",exact:true})).toBeVisible();
  await expect.poll(()=>page.evaluate(()=>new URLSearchParams(location.search).get("focus"))).toBe("preview-1-0");
  await settled(page);
  await expect.poll(()=>page.evaluate(()=>({lines:window.__mapGpu.duplicateLines,arrows:window.__mapGpu.duplicateArrows}))).toEqual({lines:2,arrows:2});
}
async function close(page:Page){await page.getByRole("dialog",{name:"선택한 자료 상세",exact:true}).getByRole("button",{name:"관리 패널 닫기",exact:true}).click();
  await expect(page.locator(".management-panel")).toHaveCount(0);
  await settled(page);}
async function cycle(page:Page){
 const changes:number[]=[];
 const measure=async(action:()=>Promise<void>)=>{const before=await page.evaluate(()=>window.__mapGpu.programs);
  await action();changes.push(await page.evaluate(()=>window.__mapGpu.programs)-before);};
 await measure(()=>expand(page));
  await measure(()=>choose(page));
  await measure(()=>close(page));
 if(!await page.getByRole("button",{name:"묶음 접기",exact:true}).count())await expand(page);
 await measure(async()=>{await page.getByRole("button",{name:"묶음 접기",exact:true}).click();
  await expect(page.locator(".graph-core-actions")).toHaveCount(0);
  await settled(page);});
 return changes;
}
test.use({viewport:{width:1440,height:900},reducedMotion:"reduce"});
test("shared GPU resources survive state changes and release on map unmount",async({page})=>{
 const fixture=createMapFixture(false),errors:string[]=[];page.on("pageerror",e=>errors.push(e.message));
 await page.addInitScript({content:`(${installGraphProbe.toString()})();(${observeGpu.toString()})();`});
 await page.route("**/*",async route=>{const url=new URL(route.request().url());if(url.origin!=="http://127.0.0.1:47839"){errors.push(`external:${url.origin}`);
  return route.abort("blockedbyclient");}if(!url.pathname.startsWith("/api/"))return route.continue();
  const response=fixture.respond(url,route.request().postData());if(!response.ok)errors.push(url.pathname);
  let body=await response.text();
  if(url.pathname==="/api/graph"){
   const snapshot=JSON.parse(body),edge=snapshot.links.find((link:any)=>link.source==="preview-1-0"&&link.target==="preview-1-1"&&link.kind==="reference"&&link.current);
   expect(edge).toBeTruthy();snapshot.links.push({...edge});
   for(const field of ["totals","returned","eligible"])snapshot[field].links++;
   body=JSON.stringify(snapshot);
  }
  return route.fulfill({status:response.status,contentType:"application/json",body});});
 await page.goto("/");
  await expect(page.locator(".graph-viewport canvas[data-engine]")).toBeVisible({timeout:30_000});
  await settled(page);
 await cycle(page);
  const changes=await cycle(page);
 expect(changes.reduce((sum,n)=>sum+n,0)).toBeLessThanOrEqual(8);
 expect(await page.evaluate(()=>window.__mapGpu.shadersDisposed)).toBe(0);
 expect(await page.evaluate(()=>window.__mapGpu.planesDisposed)).toBe(0);
 const before=await page.evaluate(()=>({...window.__mapGpu}));
  await page.getByRole("button",{name:"목록 보기",exact:true}).click();
  await expect(page.locator(".graph-list")).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>window.__mapGpu.shadersDisposed)).toBe(2);
 expect(await page.evaluate(()=>window.__mapGpu.planesDisposed)).toBe(1);
 expect(await page.evaluate(()=>window.__mapGpu.deleted)).toBeGreaterThan(before.deleted);
 await page.getByRole("button",{name:"3D 보기",exact:true}).click();
  await expect(page.locator(".graph-viewport canvas[data-engine]")).toBeVisible();
  await settled(page);
  await expand(page);
  await choose(page);
 expect(errors).toEqual([]);
});
