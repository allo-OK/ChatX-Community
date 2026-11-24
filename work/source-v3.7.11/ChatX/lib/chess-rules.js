'use strict';

function owner(piece){return piece?(piece===piece.toUpperCase()?0:1):-1;}
function findKing(board,player){return board.findIndex(x=>x===(player===0?'K':'k'));}
function attacks(board,from,to,player){
  const p=board[from];if(!p||owner(p)!==player)return false;
  const fr=Math.floor(from/8),fc=from%8,tr=Math.floor(to/8),tc=to%8,dr=tr-fr,dc=tc-fc,ar=Math.abs(dr),ac=Math.abs(dc),low=p.toLowerCase();
  const clear=(sr,sc)=>{let r=fr+sr,c=fc+sc;while(r!==tr||c!==tc){if(board[r*8+c])return false;r+=sr;c+=sc;}return true;};
  if(low==='p')return dr===(player===0?-1:1)&&ac===1;
  if(low==='n')return(ar===2&&ac===1)||(ar===1&&ac===2);
  if(low==='k')return ar<=1&&ac<=1;
  if(low==='r')return(dr===0||dc===0)&&clear(Math.sign(dr),Math.sign(dc));
  if(low==='b')return ar===ac&&clear(Math.sign(dr),Math.sign(dc));
  if(low==='q')return((dr===0||dc===0)||ar===ac)&&clear(Math.sign(dr),Math.sign(dc));
  return false;
}
function inCheck(board,player){const k=findKing(board,player);if(k<0)return false;for(let i=0;i<64;i++)if(attacks(board,i,k,1-player))return true;return false;}
function basicValid(board,from,to,player){
  if(from<0||from>63||to<0||to>63||from===to)return false;
  const p=board[from];if(!p||owner(p)!==player||(board[to]&&owner(board[to])===player)||board[to]?.toLowerCase()==='k')return false;
  const fr=Math.floor(from/8),fc=from%8,tr=Math.floor(to/8),tc=to%8,dr=tr-fr,dc=tc-fc,ar=Math.abs(dr),ac=Math.abs(dc),low=p.toLowerCase();
  const clear=(sr,sc)=>{let r=fr+sr,c=fc+sc;while(r!==tr||c!==tc){if(board[r*8+c])return false;r+=sr;c+=sc;}return true;};
  if(low==='p'){const dir=player===0?-1:1,start=player===0?6:1;return(dc===0&&!board[to]&&(dr===dir||(fr===start&&dr===2*dir&&!board[(fr+dir)*8+fc])))||(ac===1&&dr===dir&&!!board[to]);}
  if(low==='n')return(ar===2&&ac===1)||(ar===1&&ac===2);
  if(low==='k')return ar<=1&&ac<=1;
  if(low==='r')return(dr===0||dc===0)&&clear(Math.sign(dr),Math.sign(dc));
  if(low==='b')return ar===ac&&clear(Math.sign(dr),Math.sign(dc));
  if(low==='q')return((dr===0||dc===0)||ar===ac)&&clear(Math.sign(dr),Math.sign(dc));
  return false;
}
function chessLegal(board,from,to,player,rights={},enPassantTarget=-1){
  if(!Array.isArray(board)||!Number.isInteger(from)||!Number.isInteger(to)||from<0||from>63||to<0||to>63||from===to)return false;
  const piece=board[from],row=player===0?7:0,kingFrom=row*8+4,enemy=1-player;
  if(piece?.toLowerCase()==='k'&&owner(piece)===player&&from===kingFrom&&Math.floor(to/8)===row&&Math.abs(to%8-from%8)===2){
    const side=to%8===6?'KingSide':'QueenSide',key=(player===0?'white':'black')+side,rookFrom=row*8+(side==='KingSide'?7:0),step=side==='KingSide'?1:-1;
    if(rights?.[key]!==true||board[rookFrom]?.toLowerCase()!=='r'||owner(board[rookFrom])!==player||inCheck(board,player))return false;
    for(let c=from%8+step;c!==rookFrom%8;c+=step)if(board[row*8+c])return false;
    for(let c=from%8+step;c!==to%8+step;c+=step){const test=board.slice();test[row*8+c]=piece;test[from]=null;if(inCheck(test,player))return false;}
    const next=board.slice(),rookTo=row*8+(side==='KingSide'?5:3);next[to]=piece;next[from]=null;next[rookTo]=board[rookFrom];next[rookFrom]=null;return !inCheck(next,player);
  }
  const ep=piece?.toLowerCase()==='p'&&to===Number(enPassantTarget)&&!board[to]&&Math.abs(to%8-from%8)===1;
  if(ep){const dir=player===0?-1:1,captured=to-dir*8;if(board[captured]?.toLowerCase()!=='p'||owner(board[captured])!==enemy)return false;const b=board.slice();b[to]=piece;b[from]=null;b[captured]=null;return !inCheck(b,player);}
  if(!basicValid(board,from,to,player))return false;
  const b=board.slice();b[to]=b[from];b[from]=null;return !inCheck(b,player);
}
module.exports={chessLegal};
