const fs=require('fs'),path=require('path');
async function psdThumbnail(file){
 const handle=await fs.promises.open(file,'r');try{
  const size=(await handle.stat()).size;
  async function read(at,length){if(at<0||length<0||at+length>size)throw Error('Data Photoshop tidak lengkap');const b=Buffer.alloc(length);const result=await handle.read(b,0,length,at);if(result.bytesRead!==length)throw Error('Data Photoshop tidak lengkap');return b;}
  const header=await read(0,30);if(header.toString('ascii',0,4)!=='8BPS'||![1,2].includes(header.readUInt16BE(4)))return null;
  let position=30+header.readUInt32BE(26);const resources=(await read(position,4)).readUInt32BE(0);position+=4;const end=position+resources;if(end>size)return null;
  while(position+12<=end){const resource=await read(position,7);if(resource.toString('ascii',0,4)!=='8BIM')return null;const id=resource.readUInt16BE(4),nameSize=resource[6]+1;position+=6+nameSize+(nameSize%2);const length=(await read(position,4)).readUInt32BE(0);position+=4;if(position+length>end)return null;
   if((id===1033||id===1036)&&length>28&&length<=16*1024*1024){const data=await read(position,length);if(data.readUInt32BE(0)===1&&data[28]===255&&data[29]===216)return data.subarray(28);}
   position+=length+(length%2);
  }return null;
 }finally{await handle.close();}
}
async function thumbnail(file,{sharp,nativeImage}){
 const ext=path.extname(file).toLowerCase();let input=file;
 if(ext==='.psd'||ext==='.psb'){input=await psdThumbnail(file);if(!input){const error=Error('File Photoshop tidak menyimpan thumbnail. Simpan dengan preview di Photoshop atau gunakan PNG/JPG pendamping.');error.code='PREVIEW_UNAVAILABLE';throw error;}}
 try{return await sharp(input,{failOn:'none',limitInputPixels:268402689}).rotate().resize({width:260,height:260,fit:'inside',withoutEnlargement:true}).flatten({background:'#f2f2f2'}).jpeg({quality:60,mozjpeg:true}).toBuffer();}
 catch(error){if(typeof input==='string'){const image=nativeImage?.createFromPath(input);if(image&&!image.isEmpty()){const size=image.getSize(),ratio=Math.min(1,260/Math.max(size.width,size.height));return image.resize({width:Math.max(1,Math.round(size.width*ratio)),height:Math.max(1,Math.round(size.height*ratio)),quality:'good'}).toJPEG(60);}}
  if(/unsupported image|unsupported.*format|no decode delegate|invalid image/i.test(error.message)){error.code='PREVIEW_UNAVAILABLE';error.message='Format atau isi file ini belum bisa ditampilkan sebagai preview. File asli tetap dapat diunduh.';}throw error;
 }
}
module.exports={psdThumbnail,thumbnail};
