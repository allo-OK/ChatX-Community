const fs=require('fs');
const path=require('path');
const sharp=require('sharp');
const {Potrace}=require('potrace');

// v1.11.5: true multi-color vector output. Potrace is used once per quantized color layer,
// then the colored paths are overlaid into a single SVG.
const PRESETS={
  logo:{name:'Logo / Warna Solid',colors:5,turdSize:5,optTolerance:.30,maxSide:2200,minPixels:12},
  color:{name:'Vektor Standar Berwarna',colors:9,turdSize:3,optTolerance:.22,maxSide:1800,minPixels:8},
  detail:{name:'Detail Warna Halus',colors:14,turdSize:2,optTolerance:.14,maxSide:1600,minPixels:5}
};
function send(o){try{process.stdout.write(JSON.stringify(o)+'\n')}catch{}}
function hex(r,g,b){return '#'+[r,g,b].map(v=>Math.max(0,Math.min(255,v|0)).toString(16).padStart(2,'0')).join('')}
function escapeXml(s){return String(s||'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&apos;'}[c]))}
async function traceMask(mask,fill,preset){
  return await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>reject(new Error('Tracing layer melewati batas waktu')),70000);
    const tracer=new Potrace({threshold:128,blackOnWhite:true,turdSize:preset.turdSize,optTolerance:preset.optTolerance,turnPolicy:Potrace.POLICY_MINORITY,color:fill,background:'transparent'});
    tracer.loadImage(mask,err=>{
      clearTimeout(timer);if(err)return reject(err);
      try{
        const svg=tracer.getSVG();
        const paths=String(svg||'').match(/<path\b[^>]*>/gi)||[];
        const recolored=paths.map(tag=>{
          let t=tag.replace(/\sfill=("[^"]*"|'[^']*')/i,'');
          return t.replace(/\/>$/,'>').replace(/>$/,'')+` fill="${fill}"/>`;
        }).join('\n');
        resolve(recolored);
      }catch(e){reject(e)}
    });
  });
}
(async()=>{
  try{
    const payload=JSON.parse(Buffer.from(process.argv[2]||'','base64').toString('utf8')||'{}');
    const input=String(payload.input||''),originalInput=String(payload.originalInput||input),presetKey=PRESETS[payload.preset]?payload.preset:'color',preset=PRESETS[presetKey];
    if(!input||!fs.existsSync(input))throw new Error('File gambar tidak ditemukan');
    const st=fs.statSync(input);if(st.size>100*1024*1024)throw new Error('Gambar terlalu besar. Maksimal 100 MB.');
    const outDir=String(payload.outputDir||'')||path.dirname(originalInput);fs.mkdirSync(outDir,{recursive:true});
    const base=path.basename(originalInput,path.extname(originalInput)).replace(/[\\/:*?"<>|]/g,'_').slice(0,120),output=path.join(outDir,`${base}_KC-vector_${presetKey}.svg`);

    send({type:'progress',pct:8,text:'Membaca warna gambar…'});
    const meta=await sharp(input,{failOn:'none',limitInputPixels:268402689}).metadata();
    const scale=Math.min(1,preset.maxSide/Math.max(meta.width||1,meta.height||1));
    const width=Math.max(1,Math.round((meta.width||1)*scale)),height=Math.max(1,Math.round((meta.height||1)*scale));

    // libvips palette quantization preserves multiple source colors instead of converting to grayscale.
    const quantized=await sharp(input,{failOn:'none',limitInputPixels:268402689})
      .rotate().resize({width,height,fit:'fill',withoutEnlargement:true})
      .ensureAlpha().png({palette:true,colors:preset.colors,dither:0,compressionLevel:3}).toBuffer();
    const rawObj=await sharp(quantized,{failOn:'none'}).ensureAlpha().raw().toBuffer({resolveWithObject:true});
    const raw=rawObj.data,w=rawObj.info.width,h=rawObj.info.height;

    send({type:'progress',pct:20,text:`Memisahkan hingga ${preset.colors} layer warna…`});
    const counts=new Map();
    for(let i=0;i<raw.length;i+=4){if(raw[i+3]<20)continue;const key=(raw[i]<<16)|(raw[i+1]<<8)|raw[i+2];counts.set(key,(counts.get(key)||0)+1)}
    let palette=[...counts.entries()].sort((a,b)=>b[1]-a[1]).slice(0,preset.colors).filter(x=>x[1]>=preset.minPixels);
    if(!palette.length)throw new Error('Tidak ada warna yang bisa ditrace');

    const layers=[];let layerNo=0;
    for(const [key,count] of palette){
      layerNo++;
      const r=(key>>16)&255,g=(key>>8)&255,b=key&255,fill=hex(r,g,b);
      const mask=Buffer.allocUnsafe(w*h*3);
      for(let px=0,p=0;px<raw.length;px+=4,p+=3){
        const same=raw[px+3]>=20&&raw[px]===r&&raw[px+1]===g&&raw[px+2]===b;
        const v=same?0:255;mask[p]=v;mask[p+1]=v;mask[p+2]=v;
      }
      const maskPng=await sharp(mask,{raw:{width:w,height:h,channels:3}}).png({compressionLevel:1}).toBuffer();
      const pct=24+Math.round((layerNo/palette.length)*62);send({type:'progress',pct,text:`Tracing warna ${layerNo}/${palette.length} · ${fill}`});
      const pathTags=await traceMask(maskPng,fill,preset);
      if(pathTags)layers.push(`<g data-color="${fill}" data-pixels="${count}">${pathTags}</g>`);
    }
    if(!layers.length)throw new Error('Tidak ada path vector yang berhasil dibuat');
    const svg=`<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" shape-rendering="geometricPrecision">\n<title>${escapeXml(base)} · ChatX Color Vector</title>\n${layers.join('\n')}\n</svg>\n`;
    send({type:'progress',pct:93,text:`Menyimpan SVG berwarna · ${layers.length} layer…`});
    fs.writeFileSync(output,svg,'utf8');
    send({type:'done',ok:true,output,preset:presetKey,presetName:preset.name,size:fs.statSync(output).size,width:w,height:h,colorLayers:layers.length,palette:palette.map(([k,c])=>({color:hex((k>>16)&255,(k>>8)&255,k&255),pixels:c}))});
  }catch(e){send({type:'done',ok:false,error:e.message||String(e),stack:e.stack||''});process.exitCode=1;}
})();
