import { readFile, writeFile, mkdir, cp } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

export const root = fileURLToPath(new URL("../", import.meta.url));
export const frontendRoot = path.join(root, "recovered-frontend");
export const storeFile = "_app/immutable/chunks/BsV_nQF7.js";
export const screensFile = "_app/immutable/chunks/CKwrNOLH.js";

function replace(source, before, after, count = 1) {
  if (source.split(before).length - 1 !== count) {
    throw new Error("Unexpected 1.9.0 bundle at " + before);
  }
  return source.replaceAll(before, after);
}

export async function optimizedFiles() {
  const helper = (await readFile(path.join(root, "frontend/entity-index.mjs"), "utf8"))
    .replace("export function indexEntities", "function indexEntities");
  let store = await readFile(path.join(frontendRoot, storeFile), "utf8");
  // Read the reactive list on each lookup. The store replaces lists, so identity invalidates the cache.
  store = replace(store, "class _t{", helper + '\nclass _t{');
  store = replace(store, "get entities(){return p(this.#a)}",
    "get entities(){return p(this.#a)}\n" +
    "#indexedList; #entityIndex;\n" +
    "get lookupIndex(){const rows=this.entities;if(rows!==this.#indexedList){this.#entityIndex=indexEntities(rows);this.#indexedList=rows}return this.#entityIndex}\n" +
    "entityById(id){return this.lookupIndex.byId.get(id)}\n" +
    "entitiesOfType(id){return this.lookupIndex.byType.get(id)??[]}\n");
  store = replace(store, "this.entities.some(o=>o.id===r.ref)", "!!this.entityById(r.ref)");
  store = replace(store, "this.entities.find(i=>i.id===a)", "this.entityById(a)");
  let screens = await readFile(path.join(frontendRoot, screensFile), "utf8");
  const mediaUsage = (await readFile(path.join(root, "frontend/media-usage.mjs"), "utf8"))
    .replace("export function openMediaUsage", "function openMediaUsage");
  screens = mediaUsage + "\n" + screens;
  screens = replace(screens, "ke=>ke.entity_id+ke.role", "ke=>[ke.kind,ke.entity_id??ke.id,ke.role].join(':')");
  screens = replace(screens, "q.openEntity(l(Ae).entity_id)", "openMediaUsage(q,l(Ae))");
  for (const [before, after, count] of [
    ["Er=>Er.id===Cr[0]", "Cr[0]", 2], ["ye=>ye.id===Ne[0]", "Ne[0]", 1],
    ["Kt=>Kt.id===ft", "ft", 1], ["ge=>ge.id===l(o)", "l(o)", 1],
    ["ge=>ge.id===l(m)", "l(m)", 1], ["A=>A.id===E.ref", "E.ref", 1],
    ["D=>D.id===E.ref", "E.ref", 1], ["V=>V.id===l(u)[0].id", "l(u)[0].id", 1], ["Or=>Or.id===Yr","Yr",1],
  ]) screens = replace(screens, "q.entities.find("+before+")", "q.entityById("+after+")", count);
  screens = replace(screens, "q.entities.filter(F=>F.type_id===e.id)", "q.entitiesOfType(e.id)");
  screens = replace(screens, "q.entities.filter(zt=>zt.type_id===tt.entity.type_id)", "q.entitiesOfType(tt.entity.type_id)");
  screens = replace(screens, "return q.entities.filter(Ze=>Ze.type_id===tt&&(!l(h)||", "const group=q.entitiesOfType(tt);return !l(h)?group:group.filter(Ze=>(");
  const paint = (await readFile(path.join(root,"frontend/map-pixels.mjs"),"utf8"))
    .replace("export function paintGridRect", "function paintGridRect");
  screens = replace(screens, "const le=Ce(()=>{const ke=new Uint8ClampedArray(1024).fill(128);",
    paint + "\nconst le=Ce(()=>{const ke=new Uint8ClampedArray(1024).fill(128);for(let alpha=3;alpha<1024;alpha+=4)ke[alpha]=255;");
  screens = replace(screens,"Ae.id*4)}return ke});function ve(", "Ae.id*4)}return new Uint32Array(ke.buffer)});function ve(");
  screens = replace(screens,
    "for(let bt=Ae;bt<=Xe;bt++)for(let Ct=ke;Ct<=Ee;Ct++){const Ne=bt*kt+Ct,dt=Ue[Ne]*4,ye=Ne*4;ht[ye]=gt[dt],ht[ye+1]=gt[dt+1],ht[ye+2]=gt[dt+2],ht[ye+3]=255}",
    "paintGridRect(ht,Ue,kt,gt,ke,Ae,Ee,Xe);");
  return new Map([[storeFile,store],[screensFile,screens]]);
}

export async function buildFrontend() {
  const out = path.join(root, "dist", "frontend");
  await mkdir(out, {recursive:true});
  await cp(frontendRoot, out, {recursive:true});
  for (const [file, content] of await optimizedFiles()) await writeFile(path.join(out,file),content);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await buildFrontend();
