'use strict';
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { createHash, generateKeyPairSync, randomUUID, sign } = require('node:crypto');
const { once } = require('node:events');
const fs = require('node:fs');
const net = require('node:net');
const path = require('node:path');
const { createInterface } = require('node:readline');
const { setTimeout: delay } = require('node:timers/promises');

async function waitFor(read, description) {
    const deadline = Date.now() + 8000;
    while (Date.now() < deadline) {
        const result = await read();
        if (result) return result;
        await delay(80);
    }
    throw Error(description);
}

async function evaluate(target, expression, timeout = 4000) {
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    try {
        await new Promise((resolve, reject) => {
            socket.addEventListener('open', resolve, {once:true});
            socket.addEventListener('error', reject, {once:true});
        });
        return await new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(Error('Runtime.evaluate timeout')), timeout);
            socket.addEventListener('message', ({data}) => {
                const response = JSON.parse(data);
                if (response.id !== 1) return;
                clearTimeout(timer);
                if (response.error || response.result.exceptionDetails) {
                    reject(Error(JSON.stringify(response.error || response.result.exceptionDetails)));
                    return;
                }
                resolve(response.result.result.value);
            });
            socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',
                params:{expression,awaitPromise:true,returnByValue:true}}));
        });
    } finally {
        socket.close();
    }
}

async function main() {
    const executable = process.env.KC_TEST_CLIENT_EXE;
    assert(executable, 'Set KC_TEST_CLIENT_EXE');
    const hostExecutable = process.env.KC_LAB_EXE;
    assert(hostExecutable, 'Set KC_LAB_EXE');
    const server = net.createServer().listen(0,'127.0.0.1');
    await once(server,'listening');
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve));
    const profile = fs.mkdtempSync(path.join(path.dirname(executable),'required-update-qa-'));
    let client, host;
    try {
        const hostData = path.join(profile,'host');
        const feed = path.join(hostData,'updates','rust');
        fs.mkdirSync(feed,{recursive:true});
        host = spawn(hostExecutable,['--lab-host'],{cwd:hostData,windowsHide:true});
        const output = createInterface({input:host.stdout});
        const [line] = await Promise.race([
            once(output,'line'),
            once(host,'exit').then(([code])=>{throw Error('HOST exited: '+code);}),
            delay(5000).then(()=>{throw Error('HOST startup timed out');}),
        ]);
        output.close();
        const binding = JSON.parse(line);
        const origin = 'http://127.0.0.1:'+binding.port;
        const {privateKey, publicKey} = generateKeyPairSync('ed25519');
        const publicPem = publicKey.export({type:'spki',format:'pem'});
        const payload = Buffer.alloc(100128);
        const installer = 'payload-'+randomUUID()+'.exe';
        fs.writeFileSync(path.join(feed,installer),payload);
        const fields = {version:'1.1.44',size:payload.length,sha256:createHash('sha256').update(payload).digest('hex'),publishedAt:Date.now(),minProtocol:2};
        const signed = message => sign(null,Buffer.from(message),privateKey).toString('base64');
        const manifestFor = required => {
            const canonical = JSON.stringify(fields);
            const policy = JSON.stringify({manifest:canonical,required});
            return {...fields,required,publicKey:publicPem,signature:signed(canonical),
                policySignature:signed(policy),channel:'rust',channelSignature:signed('ChatX/Community/Rust-CLIENT/v1\n'+policy)};
        };
        const publishPolicy = required => {
            fields.publishedAt++;
            fs.writeFileSync(path.join(feed,'client-update.json'),JSON.stringify({
                version:fields.version,publishedAt:fields.publishedAt,installer,manifest:manifestFor(required),
            }));
        };
        fs.writeFileSync(path.join(profile,'trusted-hosts.json'),JSON.stringify({[origin]:publicPem}));
        fs.writeFileSync(path.join(profile,'required-rust-client-update.json'),JSON.stringify({origin,manifest:manifestFor(true)}));
        client = spawn(executable,['--smoke-client-renderer'],{cwd:profile,windowsHide:true,env:{...process.env,
            KC_LAB_DISCOVERY_PORT:String(binding.discoveryPort),KC_LAB_UPDATER_DIR:profile,WEBVIEW2_USER_DATA_FOLDER:path.join(profile,'webview'),
            WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-address=127.0.0.1 --remote-debugging-port=${port}`,
        }});
        const targets = async () => {
            if (client.exitCode !== null) throw Error('CLIENT exited before WebView test finished');
            try {
                return await (await fetch(`http://127.0.0.1:${port}/json/list`,{signal:AbortSignal.timeout(1000)})).json();
            } catch {
                return [];
            }
        };
        const page = (matches,description) => waitFor(async()=> (await targets()).find(t=>t.type==='page'&&matches(t.url)),description);
        const main = await page(url=>url.endsWith('/renderer/index.html'),'Main WebView missing');
        const locked = target => evaluate(target,"document.getElementById('requiredUpdate')?.open===true");
        await waitFor(()=>locked(main),'Main window did not enforce signed policy');
        await evaluate(main,`Promise.all([
            window.__TAURI__.core.invoke('open_game')
        ]).then(()=>true)`);
        const game = await page(url=>url.endsWith('/game-window.html'),'Game WebView missing');
        for (const target of [main,game]) {
            await waitFor(()=>locked(target),'Signed policy was not restored: '+target.url);
            assert(await evaluate(target,`(() => {
                const dialog=document.getElementById('requiredUpdate');
                return !dialog.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true})) && dialog.open;
            })()`));
        }
        await evaluate(main,`(async () => {
            const invoke=window.__TAURI__.core.invoke;
            await invoke('chat_test_port');
            await invoke('remember_update_host',{
                key:${JSON.stringify(publicPem)},
                fingerprint:${JSON.stringify(createHash('sha256').update(publicPem).digest('hex'))},
                token:'1'.repeat(64)
            });
            return true;
        })()`);
        for (const required of [false,true]) {
            publishPolicy(required);
            await evaluate(main,'window.kc.checkRustClientUpdate().then(()=>true)');
            for (const target of [main,game]) {
                await waitFor(async()=>await locked(target)===required,'Live policy did not reach '+target.url);
            }
        }
        const stopped = once(client,'close');
        await evaluate(main,"setTimeout(()=>void window.__TAURI__.core.invoke('finish_chat_test',{error:null}),50);true");
        const result = await Promise.race([stopped,delay(3000, null)]);
        assert.equal(result?.[0],0,'CLIENT did not close cleanly');
        console.log('PASS: signed Rust policy restores, unlocks and relocks chat and game WebViews through the HOST feed');
    } finally {
        if (client && client.exitCode === null) {
            const stopped = once(client,'close');
            client.kill();
            await stopped;
        }
        if (host && host.exitCode === null) {
            const stopped = once(host,'close');
            host.kill();
            await stopped;
        }
        await fs.promises.rm(profile,{recursive:true,force:true,maxRetries:12,retryDelay:200});
    }
}

if (require.main === module) {
    main().catch(error=>{console.error(error);process.exitCode=1;});
}
module.exports = {evaluate,waitFor};
