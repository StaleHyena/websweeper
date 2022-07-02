use std::{
    convert::TryInto,
    num::NonZeroUsize,
};
use rand::{ thread_rng, Rng, distributions::Uniform };
use serde::Serialize;

const HIDDEN_BIT: u8 = 1 << 7;
pub const FLAGGED_BIT: u8 = 1 << 6;
const CORRECT_BIT: u8 = 1 << 5; // grading for a rightly flagged mine
// all the bits that aren't flags
const TILE_NUMBITS: u8 = !(HIDDEN_BIT | FLAGGED_BIT | CORRECT_BIT);
const MINED: u8 = HIDDEN_BIT | TILE_NUMBITS;
const NEIGH_OFFS: &[(isize,isize)] = &[
    (-1,-1),(0,-1),(1,-1),
    (-1, 0),       (1, 0),
    (-1, 1),(0, 1),(1, 1),
];
#[derive(PartialEq)]
pub enum Phase {
    SafeFirstMove,
    FirstMoveFail,
    Run,
    Die,
    Win,
//    Leave,
}
pub struct Game {
    pub phase: Phase,
    pub board: Board,
    pub board_conf: BoardConf,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BoardConf {
    pub w: NonZeroUsize,
    pub h: NonZeroUsize,
    /// mines/tiles, expressed as (numerator, denominator)
    pub mine_ratio: (usize,NonZeroUsize),
    pub always_safe_first_move: bool,
    pub revealed_borders: bool,
}

impl std::fmt::Display for BoardConf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{} {}/{}", self.w, self.h, self.mine_ratio.0, self.mine_ratio.1)
    }
}

pub struct Board {
    pub data: Vec<u8>,
    pub width: NonZeroUsize,
    pub height: NonZeroUsize,
    pub hidden_tiles: usize,
    pub mine_count: usize,
}
#[derive(Debug)]
pub enum MoveType {
    Reveal,
    ToggleFlag,
}
#[derive(Debug)]
pub struct Move {
    pub t: MoveType,
    pub pos: (usize,usize),
}

pub struct MoveResult(pub Board, pub bool);
impl Game {
    pub fn new(conf: BoardConf) -> Self {
        let board = Board::new(conf);
        Game {
            phase: if conf.always_safe_first_move { Phase::SafeFirstMove } else { Phase::Run },
            board,
            board_conf: conf
        }
    }
    pub fn act(mut self, m: Move) -> Self {
        let lost_phase = | phase | {
            match phase {
                Phase::SafeFirstMove => Phase::FirstMoveFail,
                Phase::Run => Phase::Die,
                _ => unreachable!(),
            }
        };

        match m.t {
            MoveType::Reveal => {
                let kaboom: bool;
                self.board = {
                    let mr = self.board.reveal(m.pos);
                    kaboom = mr.1;
                    mr.0
                };
                if kaboom { self.phase = lost_phase(self.phase) }
                if self.phase == Phase::SafeFirstMove { self.phase = Phase::Run }
            },
            MoveType::ToggleFlag => self.board = self.board.flag(m.pos).0,
        };

        if self.phase == Phase::FirstMoveFail {
            let winnable = self.board.mine_count < (self.board.width.get() * self.board.height.get());
            if winnable {
                self.board.hidden_tiles += 1;
                self.board.move_mine_elsewhere(m.pos);
                self.phase = Phase::Run;
                self = self.act(m);
            } else {
                self.phase = Phase::Die;
            }
        } else if self.phase != Phase::Die && self.board.hidden_tiles == self.board.mine_count {
            self.phase = Phase::Win;
        }
        self
    }
}
impl Board {
    pub fn new(mut conf: BoardConf) -> Self {
        let (w,h) = (conf.w,conf.h);
        let area = w.get()*h.get();
        if w.get() < 3 || h.get() < 3 { conf.revealed_borders = false; }
        let mined_area = area - if conf.revealed_borders { 2*(w.get()-1) + 2*(h.get()-1) } else { 0 };
        let mine_count = ((conf.mine_ratio.0 * mined_area) / conf.mine_ratio.1.get()).clamp(0, mined_area);
        let b = Board {
            data: [HIDDEN_BIT].repeat(area),
            width: w,
            height: h,
            hidden_tiles: area,
            mine_count,
        };
        if conf.revealed_borders {
            let (w,h) = (w.get(),h.get());
            let mut b = b.spread_mines(mine_count, true);
            for x in 0..w {
                b = b.reveal((x,   0)).0;
                b = b.reveal((x, h-1)).0;
            }
            for y in 1..h-1 {
                b = b.reveal((  0, y)).0;
                b = b.reveal((w-1, y)).0;
            }
            b
        } else { b.spread_mines(mine_count, false) }
    }
    pub fn spread_mines(mut self, mut count: usize, without_edges: bool) -> Self {
        let mut rng = thread_rng();
        let w = self.width.get();
        let h = self.height.get();
        let (wr,hr) = if without_edges { ((1,w-1),(1,h-1)) } else { ((0,w),(0,h)) };
        while count > 0 {
            let randpos: (usize, usize) = (rng.sample(Uniform::new(wr.0, wr.1)), rng.sample(Uniform::new(hr.0, hr.1)));
            let o = self.pos_to_off_unchecked(randpos);
            if self.data[o] == MINED { continue }
            else {
                self.data[o] = MINED;
                count -= 1;
                let minepos = pos_u2i(randpos).unwrap();
                self.map_neighs(minepos, |neigh| {
                    if neigh != MINED {
                        neigh + 1
                    } else { neigh }
                });
            }
        }
        self
    }

    fn neighs<T>(&self, pos: (T,T)) -> Option<Vec<(usize,usize)>>
        where T: TryInto<isize>
    {
        if let (Ok(ox),Ok(oy)) = (pos.0.try_into(),pos.1.try_into()) {
            Some(NEIGH_OFFS
                 .iter()
                 .map(|(x,y)| (*x + ox, *y + oy)).filter_map(|p| self.bounded(p))
                 .collect())
        } else {
            None
        }
    }
    fn map_neighs<T>(&mut self, pos: (T,T), mut f: impl FnMut(u8) -> u8) where T: TryInto<isize> {
        if let Some(neighs) = self.neighs(pos) {
            let npos = neighs.iter().filter_map(|pos| self.pos_to_off(*pos)).collect::<Vec<usize>>();
            npos.iter().for_each(|o| {
                self.data[*o] = f(self.data[*o]);
            });
        }
    }

    pub fn pos_to_off(&self, pos: (usize,usize)) -> Option<usize>
    {
        self.bounded(pos).map(|x| self.pos_to_off_unchecked(x))
    }
    pub fn pos_to_off_unchecked(&self, pos: (usize, usize)) -> usize {
        pos.0 + pos.1 * self.width.get()
    }
    pub fn bounded<T>(&self, pos: (T,T)) -> Option<(usize, usize)>
        where T: TryInto<usize>
    {
        if let (Ok(x),Ok(y)) = (
            pos.0.try_into(),
            pos.1.try_into(),
        ) {
            (x < self.width.get() && y < self.height.get()).then(|| (x,y))
        } else { None }
    }
    pub fn flood_reveal(&mut self, pos: (usize,usize)) {
        let mut queue = vec![pos];
        while let Some(pos) = queue.pop() {
            if let Some(off) = self.pos_to_off(pos) {
                let c = &mut self.data[off];
                if *c & HIDDEN_BIT > 0 {
                    *c &= !(HIDDEN_BIT | FLAGGED_BIT);
                    self.hidden_tiles -= 1;
                    if *c > 0 { continue; }
                    if let Some(mut adj) = self.neighs(pos) {
                        queue.append(&mut adj);
                    }
                }
            }
        }
    }
    pub fn reveal(mut self, pos: (usize,usize)) -> MoveResult {
        if let Some(off) = self.pos_to_off(pos) {
            self.flood_reveal(pos);
            let c = self.data[off];
            MoveResult(self, (c & !(FLAGGED_BIT | CORRECT_BIT)) == TILE_NUMBITS)
        } else {
            MoveResult(self, false)
        }
    }
    pub fn grade(mut self) -> Board {
        for i in &mut self.data {
            if *i == TILE_NUMBITS | FLAGGED_BIT | HIDDEN_BIT {
                *i |= CORRECT_BIT;
            }
        }
        self
    }
    pub fn flag(mut self, pos: (usize,usize)) -> MoveResult {
        if let Some(off) = self.pos_to_off(pos) {
            self.data[off] ^= FLAGGED_BIT;
        }
        MoveResult(self, false)
    }

    pub fn render(&self) -> Vec<u8> {
        let mut ret = vec![];
        for y in 0..self.height.get() {
            for x in 0..self.width.get() {
                let c = &self.data[self.pos_to_off_unchecked((x,y))];
                match *c {
                    0 => ret.push(b' '),
                    _ if *c <= 8 => ret.push(b'0' + c),
                    _ if (*c & CORRECT_BIT) > 0 => ret.push(b'C'),
                    _ if (*c & FLAGGED_BIT) > 0 => ret.push(b'F'),
                    _ if (*c & HIDDEN_BIT) > 0 => ret.push(b'#'),
                    _ if *c == TILE_NUMBITS => ret.push(b'O'),
                    _ => ret.push(b'?'),
                }
            }
            ret.extend_from_slice(b"<br>");
        }
        ret
    }

    pub fn move_mine_elsewhere(&mut self, pos: (usize, usize)) {
        let mut surround_count = 0;
        self.map_neighs(pos, |val| {
            if (val & !FLAGGED_BIT) == MINED {
                surround_count += 1;
                val
            } else {
                val - 1
            }});
        let off = self.pos_to_off(pos).unwrap();
        let vacant_pos = {
            let v = self.data.iter()
                .enumerate()
                .filter(|(_,val)| (*val & TILE_NUMBITS) != TILE_NUMBITS)
                .map(|(p,_)| p)
                .next()
                .unwrap(); // there must be at least one
            (v%self.width.get(), v/self.width.get())
        };
        let voff = self.pos_to_off_unchecked(vacant_pos);
        debug_assert!(voff != off, "swapped mine to the same position in a FirstMoveFail/grace'd first move (???)");

        { // swap 'em (keep these together, pls kthnx (bugs were had))
            self.data[voff] |= MINED;
            self.data[off] = surround_count;
        }

        self.map_neighs(vacant_pos, |val| {
            if (val & !FLAGGED_BIT) == MINED { val } else { val + 1 }
        });
    }
}

fn pos_u2i(pos: (usize, usize)) -> Option<(isize, isize)> {
    if let (Ok(x),Ok(y)) = (pos.0.try_into(), pos.1.try_into())
    { Some((x,y)) } else { None }
}

