'use strict';
const assert = require('node:assert/strict');
const {spawn} = require('node:child_process');
const {once} = require('node:events');
const {createInterface} = require('node:readline');
const {createRequire} = require('node:module');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const {io} = createRequire(require.resolve('../source-v3.7.11/ChatX/package.json'))('socket.io-client');

async function main() {
    const binary = process.env.KC_LAB_EXE;
    assert(binary,'Set KC_LAB_EXE to the Rust CLIENT test executable');
    const directory = fs.mkdtempSync(path.join(__dirname,'status-limits-'));
    const deviceKey = 'e'.repeat(64);
    fs.writeFileSync(path.join(directory,'approved-devices.json'),JSON.stringify({
        'status-limits':{name:'Status Limits',keyHash:crypto.createHash('sha256').update(deviceKey).digest('hex')},
    }));
    const host = spawn(binary,['--lab-host'],{cwd:directory,windowsHide:true,stdio:['ignore','pipe','pipe']});
    const stopped = once(host,'close');
    stopped.catch(()=>{});
    const lines = createInterface({input:host.stdout});
    let errors = '';
    host.stderr.on('data',chunk=>{errors+=chunk;});
    const timeout = setTimeout(()=>host.kill(),120000);
    let socket;
    try {
        const [line] = await Promise.race([
            once(lines,'line'),
            stopped.then(([code])=>{throw Error('HOST stopped '+code+' '+errors);}),
        ]);
        const base = 'http://127.0.0.1:'+JSON.parse(line).port;
        socket = io(base,{transports:['websocket'],forceNew:true,reconnection:false});
        await once(socket,'connect');
        const session = await socket.timeout(3000).emitWithAck('hello',{deviceId:'status-limits',name:'Status Limits',deviceKey});
        assert.equal(session.ok,true);
        const maximum = 250*1024*1024;
        for(const size of [maximum-1,maximum,maximum+1]) {
            const boundary = 'kc-status-'+crypto.randomUUID();
            const header = Buffer.from('--'+boundary+'\r\nContent-Disposition: form-data; name="media"; filename="limit.mp4"\r\nContent-Type: video/mp4\r\n\r\n');
            const footer = Buffer.from('\r\n--'+boundary+'--\r\n');
            const block = Buffer.alloc(1024*1024,37);
            async function* body() {
                yield header;
                for(let offset=0;offset<size;offset+=block.length)yield block.subarray(0,Math.min(block.length,size-offset));
                yield footer;
            }
            const response = await fetch(base+'/status/upload',{
                method:'POST',duplex:'half',body:body(),
                headers:{Authorization:'Bearer '+session.token,'Content-Type':'multipart/form-data; boundary='+boundary,
                    'Content-Length':String(header.length+size+footer.length)},
                signal:AbortSignal.timeout(60000),
            });
            const result = await response.json();
            if(size>maximum) {
                assert.equal(response.status,400,JSON.stringify(result));
                assert.equal(result.ok,false);
                assert.match(result.error,/250 MB/);
            } else {
                assert.equal(response.status,200,JSON.stringify(result));
                assert.equal(result.media.size,size);
                assert.equal(fs.statSync(path.join(directory,'status-media',result.media.stored)).size,size);
                const range = await fetch(base+result.media.url+'?kc_token='+session.token,{headers:{Range:'bytes=-16'}});
                assert.equal(range.status,206);
                assert.deepEqual(Buffer.from(await range.arrayBuffer()),Buffer.alloc(16,37));
                const published = await socket.timeout(3000).emitWithAck('status-create',{media:result.media});
                assert.equal(published.ok,true);
                assert.equal((await socket.timeout(3000).emitWithAck('status-delete',{id:published.item.id})).ok,true);
                assert(!fs.existsSync(path.join(directory,'status-media',result.media.stored)));
            }
            assert(!fs.readdirSync(path.join(directory,'status-media')).some(name=>name.endsWith('.part')));
            console.log('PASS: Status upload '+size+' bytes '+(size>maximum?'rejects with error and cleans partial':'streams, reads tail range and deletes'));
        }
    } finally {
        socket?.disconnect();
        clearTimeout(timeout);
        lines.close();
        host.kill();
        await stopped;
    }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
