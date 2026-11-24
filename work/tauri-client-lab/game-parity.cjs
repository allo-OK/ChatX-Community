'use strict';
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { createInterface } = require('node:readline');
const { createRequire } = require('node:module');
const fs = require('node:fs');
const path = require('node:path');
const legacy = path.resolve(__dirname, '../source-v3.7.11/ChatX');
const { chessLegal } = require(path.join(legacy, 'lib/chess-rules'));
const { io } = createRequire(path.join(legacy, 'package.json'))('socket.io-client');
const binary = process.env.KC_LAB_EXE;
assert(binary, 'KC_LAB_EXE must refer to the Rust HOST build');

const directory = fs.mkdtempSync(path.join(__dirname, 'game-qa-'));
const sockets = new Set();
let processHandle;
const receive = (socket, event, check = () => true) => new Promise((resolve, reject) => {
  const timer = setTimeout(() => { socket.off(event, listener); reject(Error('Timed out: ' + event)); }, 15000);
  function listener(data) {
    if (!check(data)) return;
    clearTimeout(timer);
    socket.off(event, listener);
    resolve(data);
  }
  socket.on(event, listener);
});
const request = (socket, event, input = {}) => socket.timeout(4000).emitWithAck(event, input);

async function start() {
  const child = spawn(binary, ['--lab-host'], {cwd: directory, windowsHide: true, stdio: ['ignore','pipe','inherit']});
  processHandle = child;
  const lines = createInterface({input: child.stdout});
  try {
    const line = await Promise.race([once(lines, 'line').then(([line]) => line),
      once(child, 'exit').then(([code]) => { throw Error('HOST exited: ' + code); })]);
    return 'http://127.0.0.1:' + JSON.parse(line).port;
  } finally { lines.close(); }
}
async function stop() {
  for (const socket of sockets) socket.disconnect();
  sockets.clear();
  if (processHandle?.exitCode === null) {
    const closed = once(processHandle, 'close');
    processHandle.kill();
    await closed;
  }
  processHandle = undefined;
}
async function connect(url, name, {login = true} = {}) {
  const socket = io(url, {transports: ['websocket'], forceNew: true, reconnection: false});
  sockets.add(socket);
  await receive(socket, 'connect');
  if (login) await hello(socket,name);
  return socket;
}
async function hello(socket,name) {
  const result = await request(socket, 'hello', {deviceId:name, name, deviceKey:'a'.repeat(64)});
  assert.equal(result.ok, true, JSON.stringify(result));
}
function compareMoves(state, rights, enPassant) {
  const expected = [];
  for (let from=0; from<64; from++) for (let to=0; to<64; to++) {
    if (chessLegal(state.board,from,to,state.turn,rights,enPassant)) expected.push([from,to]);
  }
  assert.deepEqual(state.legalMoves, expected);
}

function seedChessRooms() {
  const rights = {whiteKingSide:true,whiteQueenSide:true,blackKingSide:true,blackQueenSide:true};
  const room = (id, pieces, enPassantTarget = -1) => {
    const board = Array(64).fill(null);
    for (const [square,piece] of Object.entries(pieces)) board[Number(square)] = piece;
    return {id,type:'chess',status:'playing',host:'RuleWhite',board,turn:0,castlingRights:rights,enPassantTarget,
      savedAt:Date.now(),invitedNames:[],declinedNames:[],players:[{name:'RuleWhite',color:'white'},{name:'RuleBlack',color:'black'}]};
  };
  fs.writeFileSync(path.join(directory,'games.json'),JSON.stringify([
    room('qa-castling',{4:'k',56:'R',60:'K',63:'R'}),
    room('qa-en-passant',{4:'k',27:'p',28:'P',60:'K'},19),
    room('qa-promotion',{4:'k',8:'P',60:'K'}),
    room('qa-mate',{7:'k',13:'K',14:'Q'}),
    {...room('qa-idle',{4:'k',60:'K',52:'P',12:'p'}),idleMisses:{rulewhite:2}},
    {id:'qa-bot-ludo',type:'ludo',status:'playing',host:'RuleWhite',turn:0,savedAt:Date.now(),invitedNames:[],declinedNames:[],
      players:[{name:'Bot Ceria',isBot:true,color:'red'},{name:'RuleWhite',isBot:false,color:'green'}],
      ludo:{pawns:[[20,20,20,20],[-1,-1,-1,-1],[-1,-1,-1,-1],[-1,-1,-1,-1]]}},
    {id:'qa-bot-snakes',type:'snakes',status:'playing',host:'RuleWhite',turn:0,savedAt:Date.now(),invitedNames:[],declinedNames:[],
      players:[{name:'Bot Ceria',isBot:true},{name:'RuleWhite',isBot:false}],snakesLadders:{positions:[0,0],
        board:{snakes:{16:6,48:26,49:11,56:53,62:19,64:60,87:24,93:73,95:75,98:78},
          ladders:{1:38,4:14,9:31,21:42,28:84,36:44,51:67,71:91,80:100}}}},
  ]));
  return rights;
}

async function specialMoves(url, rights) {
  const white = await connect(url,'RuleWhite');
  const get = async id => (await request(white,'game-state-request',{id})).state;
  for (const [id,from,to,ep] of [
    ['qa-castling',60,62,-1],['qa-en-passant',28,19,19],['qa-promotion',8,0,-1],['qa-mate',14,6,-1],
  ]) {
    const before = await get(id);
    compareMoves(before,rights,ep);
    const move = await request(white,'chess-move',{id,from,to});
    assert.equal(move.ok,true,JSON.stringify(move));
    const after = await get(id);
    assert.equal(after.board[from],null);
    if (id === 'qa-castling') {
      assert.equal(after.board[62],'K');
      assert.equal(after.board[61],'R');
      assert.equal(after.board[63],null);
    } else if (id === 'qa-en-passant') {
      assert.equal(after.board[19],'P');
      assert.equal(after.board[27],null);
      assert.equal(after.lastAction.captured,'p');
    } else if (id === 'qa-promotion') assert.equal(after.board[0],'Q');
    else {
      assert.equal(after.status,'finished');
      assert.equal(after.winner,'RuleWhite');
    }
    await request(white,'game-cancel',{id});
  }
  white.disconnect();
  console.log('PASS: legacy game saves restore and chess castling, en passant, promotion, and checkmate resolve correctly');
}

async function automaticTurns(url) {
  const white = await connect(url,'RuleWhite');
  const idle = new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(Error('Idle cancellation did not arrive')),35000);
    white.on('game-closed',event => {
      if(event.id !== 'qa-idle') return;
      clearTimeout(timeout);
      resolve(event);
    });
  });
  // Observe failures immediately while the other multiplayer cases execute.
  idle.catch(() => {});
  async function observe(id, predicate) {
    for (let attempt=0;attempt<40;attempt++) {
      const {state} = await request(white,'game-state-request',{id});
      if (state && predicate(state)) return state;
      await new Promise(resolve=>setTimeout(resolve,100));
    }
    throw Error('Automatic turn did not arrive: ' + id);
  }
  await Promise.all([
    observe('qa-bot-ludo', state=>state.ludo.pawns[0].some(pawn=>pawn>20)),
    observe('qa-bot-snakes', state=>state.turn===1 && state.snakesLadders.positions[0]>0),
  ]);
  for (const id of ['qa-bot-ludo','qa-bot-snakes']) assert.equal((await request(white,'game-cancel',{id})).ok,true);
  return {white,idle};
}

async function main() {
  const savedRights = seedChessRooms();
  let url = await start();
  await specialMoves(url,savedRights);
  const automation = await automaticTurns(url);
  const alice = await connect(url,'Alice');
  let bob = await connect(url,'Bob');
  const other = await connect(url,'Other');
  assert.match((await request(alice,'game-create',{type:'slither'})).error,/dihapus/);
  assert.match((await request(alice,'game-create',{invitees:['Offline']})).error,/tidak online/);
  const invited = receive(bob,'game-invite');
  const chess = await request(alice,'game-create',{type:'chess',color:'white',invitees:['Bob']});
  assert.equal(chess.ok,true);
  assert.equal((await invited).id,chess.id);
  assert.equal((await request(other,'game-join',{id:chess.id})).ok,false);
  bob.disconnect();
  bob = await connect(url,'Bob',{login:false});
  const pending = receive(bob,'game-invite',invite=>invite.id===chess.id);
  await hello(bob,'Bob');
  await pending;
  assert.equal((await request(bob,'game-decline',{id:chess.id})).ok,true);
  const joined = await request(bob,'game-join',{id:chess.id});
  assert.equal(joined.state.status,'playing');
  assert.deepEqual(joined.state.declinedNames,[]);
  const rights = {whiteKingSide:true,whiteQueenSide:true,blackKingSide:true,blackQueenSide:true};
  compareMoves(joined.state,rights,-1);
  assert.equal((await request(bob,'chess-move',{id:chess.id,from:12,to:28})).ok,false);
  assert.equal((await request(alice,'chess-move',{id:chess.id,from:52,to:20})).ok,false);
  assert.equal((await request(alice,'chess-move',{id:chess.id,from:52,to:36})).ok,true);
  const state = (await request(bob,'game-state-request',{id:chess.id})).state;
  assert.equal(state.board[36],'P');
  compareMoves(state,rights,44);
  const consent = receive(bob,'game-restart-consent');
  assert.equal((await request(alice,'game-restart-request',{id:chess.id})).pending,true);
  assert.equal((await consent).from,'Alice');
  assert.equal((await request(bob,'game-restart-request',{id:chess.id})).restarted,true);
  assert.equal((await request(other,'game-state-request',{id:chess.id})).ok,false);
  const snakes = await request(alice,'game-create',{type:'snakes',invitees:['Bob']});
  const snakesState = (await request(bob,'game-join',{id:snakes.id})).state;
  assert.equal(snakesState.status,'playing');
  const roll = {id:snakes.id,clientActionId:'qa-roll-1'};
  const rolled = await request(alice,'snakes-roll',roll);
  assert.equal(rolled.to,snakesState.snakesLadders.board.ladders[rolled.dice] || rolled.dice);
  assert.equal((await request(alice,'snakes-roll',roll)).duplicate,true);
  const ludo = await request(alice,'game-create',{type:'ludo',color:'red',invitees:['Bob']});
  const ludoJoined = (await request(bob,'game-join',{id:ludo.id,color:'red'})).state;
  assert.deepEqual(ludoJoined.players.map(player=>player.color),['red','green']);
  assert.equal(ludoJoined.countdown,10);
  await receive(alice,'game-state',state=>state.id===ludo.id && state.status==='playing');
  const ludoRoll = await request(alice,'ludo-roll',{id:ludo.id,clientActionId:'qa-ludo-roll',power:88});
  assert.equal(ludoRoll.ok,true);
  if(ludoRoll.dice===6) {
    assert.deepEqual(ludoRoll.legal,[0,1,2,3]);
    assert.equal((await request(alice,'ludo-move',{id:ludo.id,pawn:0,clientActionId:'qa-ludo-move'})).ok,true);
    assert.equal((await request(alice,'game-state-request',{id:ludo.id})).state.ludo.pawns[0][0],0);
  } else {
    assert.deepEqual(ludoRoll.legal,[]);
    await receive(alice,'game-state',state=>state.id===ludo.id && state.turn===1 && state.dice===0);
  }
  const solo = await request(other,'game-create',{type:'chess',color:'black',botLevel:'hard'});
  assert.equal(solo.state.soloCountdown,10);
  assert.equal((await request(other,'game-add-bot',{id:solo.id})).ok,true);
  await receive(other,'game-state',state=>state.id===solo.id && state.turn===1 && state.lastAction?.type==='chess-move');
  assert.equal((await automation.idle).reason,'idle-3x');
  assert.equal((await request(automation.white,'game-state-request',{id:'qa-idle'})).ok,false);
  console.log('PASS: real EXE timer advances Ludo and snakes bots and cancels on the third missed turn');
  await stop();
  url = await start();
  const recovered = await connect(url,'Alice',{login:false});
  const reconnect = receive(recovered,'game-reconnect');
  await hello(recovered,'Alice');
  const rooms = await reconnect;
  assert(rooms.some(state=>state.id===chess.id && state.board[52]==='P'));
  assert(rooms.some(state=>state.id===snakes.id && state.snakesLadders.positions[0]===rolled.to));
  assert(rooms.some(state=>state.id===ludo.id && state.status==='playing'));
  const closed = receive(recovered,'game-closed',event=>event.id===chess.id);
  assert.equal((await request(recovered,'game-cancel',{id:chess.id})).ok,true);
  assert.equal((await closed).reason,'host-cancel');
  console.log('PASS: game invitation and re-invitation, chess move parity and restart consent, snakes roll dedup, Ludo countdown and turn, chess bot, persistence and cancel');
}

main().finally(stop).catch(error => {console.error(error);process.exitCode=1;});
