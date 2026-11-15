import assert from "node:assert/strict";
import {performance} from "node:perf_hooks";
import {paintGridRect} from "../frontend/map-pixels.mjs";
const width = 2048, height = 1024;
const cells = Uint8Array.from({length:width*height}, (_,i)=>(19*i + (i>>>8))&255);
const bytes = Uint8Array.from({length:1024}, (_,i)=>i%4===3?255:(47*i)&255);
const palette = new Uint32Array(bytes.buffer);
const first = new Uint8ClampedArray(cells.length*4), second = first.slice();
function original(left,top,right,bottom) {
  for(let y=top;y<=bottom;y++) for(let x=left;x<=right;x++){
    const pos=y*width+x, source=cells[pos]*4, dst=pos*4;
    first[dst]=bytes[source];first[dst+1]=bytes[source+1];first[dst+2]=bytes[source+2];first[dst+3]=255;
  }
}
function optimized(...rect){paintGridRect(second,cells,width,palette,...rect)}
for (const rect of [[7,2,7,2],[17,21,39,78],[0,0,width-1,height-1]]) {
  first.fill(100);second.fill(100); original(...rect);optimized(...rect);assert.deepEqual(second,first);
}
const times=[[],[]];
for(let round=0;round<35;round++){
  for(const n of round%2?[1,0]:[0,1]){
    const start=performance.now();(n?optimized:original)(0,0,width-1,height-1);
    if(round>=5)times[n].push(performance.now()-start);
  }
}
console.log(JSON.stringify({width,height,repetitions:30,medianMs:times.map(s=>s.toSorted((a,b)=>a-b)[15])}));
