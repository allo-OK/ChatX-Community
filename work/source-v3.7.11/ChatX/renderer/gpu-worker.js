import { removeBackground } from '/imgly/index.mjs';

const api=window.kcGpuWorker;
const chunkString=u8=>{let s='';const step=0x8000;for(let i=0;i<u8.length;i+=step)s+=String.fromCharCode(...u8.subarray(i,i+step));return btoa(s)};
const fromB64=s=>{const bin=atob(String(s||''));const out=new Uint8Array(bin.length);for(let i=0;i<bin.length;i++)out[i]=bin.charCodeAt(i);return out};
const gpuCaps={webgpu:!!navigator.gpu,webgl2:false};
try{gpuCaps.webgl2=!!document.createElement('canvas').getContext('webgl2',{premultipliedAlpha:false,preserveDrawingBuffer:true});}catch{}
api.ready(gpuCaps);

async function gpuPosterize(base64,levels=5,maxSide=1600){
  let bitmap,gl,prog,vb,tex;const shaders=[];try{
  const bytes=fromB64(base64),blob=new Blob([bytes],{type:'image/png'});bitmap=await createImageBitmap(blob);
  const scale=Math.min(1,Math.max(64,Number(maxSide)||1600)/Math.max(bitmap.width,bitmap.height));
  const w=Math.max(1,Math.round(bitmap.width*scale)),h=Math.max(1,Math.round(bitmap.height*scale));
  const canvas=document.createElement('canvas');canvas.width=w;canvas.height=h;
  gl=canvas.getContext('webgl2',{premultipliedAlpha:false,preserveDrawingBuffer:true,alpha:true});
  if(!gl)throw new Error('WebGL2 GPU tidak tersedia');
  const vs=`#version 300 es\nin vec2 p;out vec2 uv;void main(){uv=(p+1.0)*0.5;gl_Position=vec4(p,0,1);}`;
  const fs=`#version 300 es\nprecision highp float;in vec2 uv;uniform sampler2D tex;uniform float levels;out vec4 outColor;void main(){vec4 c=texture(tex,vec2(uv.x,1.0-uv.y));float n=max(2.0,levels);vec3 q=floor(c.rgb*(n-1.0)+0.5)/(n-1.0);outColor=vec4(q,c.a);}`;
  const compile=(type,src)=>{const sh=gl.createShader(type);shaders.push(sh);gl.shaderSource(sh,src);gl.compileShader(sh);if(!gl.getShaderParameter(sh,gl.COMPILE_STATUS))throw new Error(gl.getShaderInfoLog(sh)||'Shader gagal');return sh};
  prog=gl.createProgram();gl.attachShader(prog,compile(gl.VERTEX_SHADER,vs));gl.attachShader(prog,compile(gl.FRAGMENT_SHADER,fs));gl.linkProgram(prog);if(!gl.getProgramParameter(prog,gl.LINK_STATUS))throw new Error(gl.getProgramInfoLog(prog)||'Program GPU gagal');gl.useProgram(prog);
  const verts=new Float32Array([-1,-1,1,-1,-1,1,-1,1,1,-1,1,1]);vb=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,vb);gl.bufferData(gl.ARRAY_BUFFER,verts,gl.STATIC_DRAW);const loc=gl.getAttribLocation(prog,'p');gl.enableVertexAttribArray(loc);gl.vertexAttribPointer(loc,2,gl.FLOAT,false,0,0);
  tex=gl.createTexture();gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,tex);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL,false);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA,gl.RGBA,gl.UNSIGNED_BYTE,bitmap);gl.uniform1i(gl.getUniformLocation(prog,'tex'),0);gl.uniform1f(gl.getUniformLocation(prog,'levels'),Math.max(2,Math.min(8,Number(levels)||5)));gl.viewport(0,0,w,h);gl.drawArrays(gl.TRIANGLES,0,6);
  const pixels=new Uint8Array(w*h*4);gl.readPixels(0,0,w,h,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
  // Flip GPU framebuffer vertically into a normal PNG.
  const flipped=new Uint8ClampedArray(pixels.length),row=w*4;for(let y=0;y<h;y++)flipped.set(pixels.subarray((h-1-y)*row,(h-y)*row),y*row);
  const out=document.createElement('canvas');out.width=w;out.height=h;const ctx=out.getContext('2d');ctx.putImageData(new ImageData(flipped,w,h),0,0);const outBlob=await new Promise((res,rej)=>out.toBlob(b=>b?res(b):rej(new Error('GPU PNG gagal dibuat')),'image/png'));
  return {base64:chunkString(new Uint8Array(await outBlob.arrayBuffer())),width:w,height:h,backend:'GPU · WebGL2 posterize'};
  }finally{bitmap?.close();if(gl){if(tex)gl.deleteTexture(tex);if(vb)gl.deleteBuffer(vb);if(prog)gl.deleteProgram(prog);for(const sh of shaders)gl.deleteShader(sh);gl.getExtension('WEBGL_lose_context')?.loseContext();}}
}

api.onTask(async task=>{
  const id=String(task?.id||''),type=String(task?.type||'');
  try{
    if(type==='capabilities')return api.reply({id,ok:true,caps:gpuCaps});
    if(type==='vector-prep'){
      const r=await gpuPosterize(task.base64,task.levels,task.maxSide);return api.reply({id,ok:true,...r});
    }
    if(type==='remove-bg'){
      if(!navigator.gpu)throw new Error('WebGPU tidak tersedia pada GPU/driver ini');
      const bytes=fromB64(task.base64),input=new Blob([bytes],{type:'image/png'});
      const started=performance.now();
      const modelMap={small:'isnet_quint8',medium:'isnet_fp16',large:'isnet'};
      const gpuModel=modelMap[String(task.model||'medium')]||'isnet_fp16';
      const result=await removeBackground(input,{device:'gpu',model:gpuModel,debug:false,proxyToWorker:false,progress:(key,current,total)=>{const pct=total?Math.max(5,Math.min(88,Math.round(current/total*83)+5)):10;api.progress({id,pct,text:'GPU WebGPU · '+String(key||'model')});},output:{format:'image/png',quality:1}});
      const data=new Uint8Array(await result.arrayBuffer());
      return api.reply({id,ok:true,base64:chunkString(data),elapsedMs:Math.round(performance.now()-started),backend:'GPU · WebGPU / ONNX Runtime'});
    }
    throw new Error('GPU task tidak dikenal: '+type);
  }catch(e){api.reply({id,ok:false,error:e?.message||String(e),stack:e?.stack||''});}
});
