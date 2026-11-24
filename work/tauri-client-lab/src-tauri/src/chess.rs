use serde_json::{json, Value};
pub type Board = [u8; 64];
pub fn initial() -> Board {
    *b"rnbqkbnrpppppppp................................PPPPPPPPRNBQKBNR"
}
pub fn board(value: &Value) -> Option<Board> {
    let rows = value.as_array().filter(|rows| rows.len() == 64)?;
    let mut board = [b'.'; 64];
    for (i, piece) in rows.iter().enumerate() {
        if piece.is_null() {
            continue;
        }
        let piece = piece.as_str()?.as_bytes();
        if piece.len() != 1 || !b"prnbqkPRNBQK".contains(&piece[0]) {
            return None;
        }
        board[i] = piece[0];
    }
    Some(board)
}
pub fn value(board: &Board) -> Value {
    json!(board
        .iter()
        .map(|piece| if *piece == b'.' {
            None
        } else {
            Some((*piece as char).to_string())
        })
        .collect::<Vec<_>>())
}
fn owner(piece: u8) -> i32 {
    if piece == b'.' {
        -1
    } else if piece.is_ascii_uppercase() {
        0
    } else {
        1
    }
}
fn clear(board: &Board, from: usize, to: usize) -> bool {
    let (fr, fc) = (from as i32 / 8, from as i32 % 8);
    let (tr, tc) = (to as i32 / 8, to as i32 % 8);
    let (sr, sc) = ((tr - fr).signum(), (tc - fc).signum());
    let (mut r, mut c) = (fr + sr, fc + sc);
    while r != tr || c != tc {
        if !(0..8).contains(&r) || !(0..8).contains(&c) || board[(r * 8 + c) as usize] != b'.' {
            return false;
        }
        r += sr;
        c += sc;
    }
    true
}
fn shape(board: &Board, from: usize, to: usize, player: i32, attack: bool) -> bool {
    let p = board[from];
    if owner(p) != player {
        return false;
    }
    let (fr, fc, tr, tc) = (
        from as i32 / 8,
        from as i32 % 8,
        to as i32 / 8,
        to as i32 % 8,
    );
    let (dr, dc) = (tr - fr, tc - fc);
    let (ar, ac) = (dr.abs(), dc.abs());
    match p.to_ascii_lowercase() {
        b'p' => {
            let dir = if player == 0 { -1 } else { 1 };
            if attack {
                dr == dir && ac == 1
            } else {
                (dc == 0
                    && board[to] == b'.'
                    && (dr == dir
                        || (fr == if player == 0 { 6 } else { 1 }
                            && dr == 2 * dir
                            && board[((fr + dir) * 8 + fc) as usize] == b'.')))
                    || (ac == 1 && dr == dir && board[to] != b'.')
            }
        }
        b'n' => (ar == 2 && ac == 1) || (ar == 1 && ac == 2),
        b'k' => ar <= 1 && ac <= 1,
        b'r' => (dr == 0 || dc == 0) && clear(board, from, to),
        b'b' => ar == ac && clear(board, from, to),
        b'q' => (dr == 0 || dc == 0 || ar == ac) && clear(board, from, to),
        _ => false,
    }
}
pub fn in_check(board: &Board, player: i32) -> bool {
    let king = if player == 0 { b'K' } else { b'k' };
    board
        .iter()
        .position(|p| *p == king)
        .is_some_and(|king| (0..64).any(|from| shape(board, from, king, 1 - player, true)))
}
pub fn check_info(board: &Board) -> Value {
    for player in 0..2 {
        if let Some(king) = board
            .iter()
            .position(|p| *p == if player == 0 { b'K' } else { b'k' })
        {
            let checkers: Vec<_> = (0..64)
                .filter(|from| shape(board, *from, king, 1 - player, true))
                .collect();
            if !checkers.is_empty() {
                return json!({"player":player,"king":king,"checkers":checkers});
            }
        }
    }
    json!({"player":-1,"king":-1,"checkers":[]})
}
pub fn legal(board: &Board, from: usize, to: usize, player: i32, rights: &Value, ep: i64) -> bool {
    if from >= 64 || to >= 64 || from == to || !(0..2).contains(&player) {
        return false;
    }
    let piece = board[from];
    let row = if player == 0 { 7 } else { 0 };
    let king_from = row * 8 + 4;
    if piece.eq_ignore_ascii_case(&b'k')
        && owner(piece) == player
        && from == king_from
        && to / 8 == row
        && to.abs_diff(from) == 2
    {
        let king_side = to % 8 == 6;
        let key = match (player, king_side) {
            (0, true) => "whiteKingSide",
            (0, false) => "whiteQueenSide",
            (_, true) => "blackKingSide",
            _ => "blackQueenSide",
        };
        let rook = row * 8 + if king_side { 7 } else { 0 };
        if rights[key] != true
            || !board[rook].eq_ignore_ascii_case(&b'r')
            || owner(board[rook]) != player
            || in_check(board, player)
        {
            return false;
        }
        let step = if king_side { 1 } else { -1 };
        let mut column = from as i32 % 8 + step;
        while column != rook as i32 % 8 {
            if board[row * 8 + column as usize] != b'.' {
                return false;
            }
            column += step;
        }
        let mut column = from as i32 % 8 + step;
        while column != to as i32 % 8 + step {
            let mut test = *board;
            test[row * 8 + column as usize] = piece;
            test[from] = b'.';
            if in_check(&test, player) {
                return false;
            }
            column += step;
        }
        let mut next = *board;
        next[to] = piece;
        next[from] = b'.';
        next[row * 8 + if king_side { 5 } else { 3 }] = board[rook];
        next[rook] = b'.';
        return !in_check(&next, player);
    }
    if piece.eq_ignore_ascii_case(&b'p')
        && to as i64 == ep
        && board[to] == b'.'
        && (to % 8).abs_diff(from % 8) == 1
    {
        let captured = to as i64 - if player == 0 { -8 } else { 8 };
        if !(0..64).contains(&captured)
            || !board[captured as usize].eq_ignore_ascii_case(&b'p')
            || owner(board[captured as usize]) != 1 - player
        {
            return false;
        }
        let mut next = *board;
        next[to] = piece;
        next[from] = b'.';
        next[captured as usize] = b'.';
        return !in_check(&next, player);
    }
    if owner(piece) != player
        || owner(board[to]) == player
        || board[to].eq_ignore_ascii_case(&b'k')
        || !shape(board, from, to, player, false)
    {
        return false;
    }
    let mut next = *board;
    next[to] = piece;
    next[from] = b'.';
    !in_check(&next, player)
}
pub fn moves(board: &Board, player: i32, rights: &Value, ep: i64) -> Vec<[usize; 2]> {
    (0..64)
        .flat_map(|from| {
            (0..64).filter_map(move |to| {
                legal(board, from, to, player, rights, ep).then_some([from, to])
            })
        })
        .collect()
}
pub fn apply(game: &mut Value, from: usize, to: usize, player: usize, action_id: &str, now: u64) {
    let mut board = board(&game["board"]).expect("validated chess board");
    let moving = board[from];
    let mut captured = board[to];
    if !game["castlingRights"].is_object() {
        game["castlingRights"] = rights();
    }
    let ep = moving.eq_ignore_ascii_case(&b'p')
        && to as i64 == game["enPassantTarget"].as_i64().unwrap_or(-1)
        && captured == b'.'
        && (to % 8).abs_diff(from % 8) == 1;
    let rights = &mut game["castlingRights"];
    if ep {
        let at = if player == 0 { to + 8 } else { to - 8 };
        captured = board[at];
        board[at] = b'.';
    }
    let castling = moving.eq_ignore_ascii_case(&b'k') && to.abs_diff(from) == 2;
    board[to] = moving;
    board[from] = b'.';
    if castling {
        let row = if player == 0 { 7 } else { 0 };
        let king_side = to % 8 == 6;
        let rook = row * 8 + if king_side { 7 } else { 0 };
        let target = row * 8 + if king_side { 5 } else { 3 };
        board[target] = board[rook];
        board[rook] = b'.';
    }
    if moving.eq_ignore_ascii_case(&b'k') {
        for key in if player == 0 {
            ["whiteKingSide", "whiteQueenSide"]
        } else {
            ["blackKingSide", "blackQueenSide"]
        } {
            rights[key] = json!(false);
        }
    }
    for (square, key) in [
        (56, "whiteQueenSide"),
        (63, "whiteKingSide"),
        (0, "blackQueenSide"),
        (7, "blackKingSide"),
    ] {
        if (moving.eq_ignore_ascii_case(&b'r') && from == square)
            || (captured.eq_ignore_ascii_case(&b'r') && to == square)
        {
            rights[key] = json!(false);
        }
    }
    let ep = if moving.eq_ignore_ascii_case(&b'p') && to.abs_diff(from) == 16 {
        ((from + to) / 2) as i64
    } else {
        -1
    };
    if board[to].eq_ignore_ascii_case(&b'p') && (to / 8 == 0 || to / 8 == 7) {
        board[to] = if player == 0 { b'Q' } else { b'q' };
    }
    let next = 1 - player;
    let finished = moves(&board, next as i32, rights, ep).is_empty();
    game["enPassantTarget"] = json!(ep);
    game["turn"] = json!(next);
    game["board"] = value(&board);
    game["lastAction"] = json!({"id":action_id,"type":"chess-move","player":player,"from":from,"to":to,"piece":(moving as char).to_string(),
        "captured":if captured==b'.'{None}else{Some((captured as char).to_string())},"castling":castling,"ts":now});
    if finished {
        game["status"] = json!("finished");
        let checked = in_check(&board, next as i32);
        game["winner"] = if checked {
            game["players"][player]["name"].clone()
        } else {
            Value::Null
        };
        game["draw"] = json!(!checked);
    }
}
pub fn rights() -> Value {
    json!({"whiteKingSide":true,"whiteQueenSide":true,"blackKingSide":true,"blackQueenSide":true})
}

fn piece_value(piece: u8) -> f64 {
    match piece.to_ascii_lowercase() {
        b'p' => 100.0,
        b'n' => 320.0,
        b'b' => 330.0,
        b'r' => 500.0,
        b'q' => 900.0,
        b'k' => 20000.0,
        _ => 0.0,
    }
}
fn evaluate(board: &Board, player: i32) -> f64 {
    let mut score = 0.0;
    for (i, piece) in board
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, p)| *p != b'.')
    {
        let own = owner(piece) == player;
        score += if own {
            piece_value(piece)
        } else {
            -piece_value(piece)
        };
        if own && (2..=5).contains(&(i / 8)) && (2..=5).contains(&(i % 8)) {
            score += 6.0;
        }
    }
    if in_check(board, 1 - player) {
        score += 28.0;
    }
    if in_check(board, player) {
        score -= 40.0;
    }
    score
}
fn after(board: &Board, from: usize, to: usize) -> Board {
    let mut board = *board;
    let p = board[from];
    board[to] = p;
    board[from] = b'.';
    if p.eq_ignore_ascii_case(&b'p') && (to / 8 == 0 || to / 8 == 7) {
        board[to] = if p.is_ascii_uppercase() { b'Q' } else { b'q' };
    }
    board
}
pub fn bot_move(game: &Value, player: i32) -> Option<[usize; 2]> {
    let board = board(&game["board"])?;
    let rights = &game["castlingRights"];
    let choices = moves(
        &board,
        player,
        rights,
        game["enPassantTarget"].as_i64().unwrap_or(-1),
    );
    if choices.is_empty() {
        return None;
    }
    let index = |len: usize| (uuid::Uuid::new_v4().as_u128() % len as u128) as usize;
    let level = game["botLevel"].as_str().unwrap_or("medium");
    if level == "easy" {
        return Some(choices[index(choices.len())]);
    }
    let mut scored: Vec<_> = choices
        .into_iter()
        .map(|[from, to]| {
            let next = after(&board, from, to);
            let mut score = evaluate(&next, player) + piece_value(board[to]) * 0.65;
            if level == "hard" {
                let replies = moves(&next, 1 - player, rights, -1);
                let worst = if replies.is_empty() {
                    if in_check(&next, 1 - player) {
                        100000.0
                    } else {
                        0.0
                    }
                } else {
                    replies
                        .into_iter()
                        .map(|[a, b]| evaluate(&after(&next, a, b), player))
                        .fold(f64::INFINITY, f64::min)
                };
                score = score * 0.35 + worst * 0.65;
            }
            ([from, to], score)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let count = if level == "hard" {
        scored
            .iter()
            .take_while(|(_, score)| (*score - scored[0].1).abs() < 0.01)
            .count()
    } else {
        4.min(scored.len())
    };
    Some(scored[index(count)].0)
}
