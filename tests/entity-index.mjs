import { strict as assert } from "node:assert";
import { performance } from "node:perf_hooks";
import { indexEntities } from "../frontend/entity-index.mjs";

const rows = Array.from({length:5000}, (_, i) => ({
  id: "entity-"+i, type_id: "type-"+(i%64), name: "entry "+i, summary:"", tags:[]
}));
const types = Array.from({length:64}, (_,i)=>"type-"+i);
const scan = () => types.map(type=>rows.filter(row=>row.type_id===type));
const indexed = () => {
  const index = indexEntities(rows); // Include the cost of building the index.
  return types.map(type=>index.byType.get(type)??[]);
};
assert.deepEqual(scan(),indexed());
for (const list of [[],rows,rows.toReversed(),rows.slice(0,20),[rows[0],{...rows[0]}]]) {
  const index = indexEntities(list);
  for (const id of ["missing",rows[0].id,rows[19].id]) assert.equal(index.byId.get(id),list.find(e=>e.id===id));
}
const times = [[],[]];
for(let round=0;round<65;round++){
  for(const n of round%2?[1,0]:[0,1]){
    const start=performance.now();
    const result=(n?indexed:scan)();
    assert.equal(result.length,64);
    if(round>=5)times[n].push(performance.now()-start);
  }
}
const median=s=>s.toSorted((a,b)=>a-b)[Math.floor(s.length/2)];
console.log(JSON.stringify({rows:rows.length,types:64,repetitions:60,scanMedianMs:median(times[0]),indexMedianMs:median(times[1])}));
