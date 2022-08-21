use std::{
    convert::TryInto,
    num::NonZeroUsize,
};
use rand::{ thread_rng, Rng, distributions::Uniform };
use serde::Serialize;

const HIDDEN_BIT: u8 = 1 << 7;
pub const FLAGGED_BIT: u8 = 1 << 6;
const SPECIAL_BIT: u8 = 1 << 5; // grading for a rightly flagged mine, or the question flag
// all the bits that aren't flags
const NUMBITS: u8 = !(HIDDEN_BIT | FLAGGED_BIT | SPECIAL_BIT);
const MINED: u8 = HIDDEN_BIT | NUMBITS;
const QUESTION: u8 = FLAGGED_BIT | SPECIAL_BIT;
const CORRECT: u8 = MINED | SPECIAL_BIT;

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
    pub reveal_on_lose: bool,
    pub num_tile_reveal: bool,
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
    pub num_tile_reveal: bool,
}
#[derive(Debug)]
pub enum MoveType {
    Reveal,
    ToggleFlag,
}
#[derive(Debug)]
pub struct Move {
    pub t: MoveType,
    pub pos: BoardPos,
}

impl Game {
    pub fn new(conf: BoardConf) -> Self {
        let board = Board::new(conf);
        Game {
            phase: if conf.always_safe_first_move { Phase::SafeFirstMove } else { Phase::Run },
            board,
            board_conf: conf
        }
    }
    pub fn act(&mut self, m: Move) {
        let lost_phase = | phase: &Phase | {
            match *phase {
                Phase::SafeFirstMove => Phase::FirstMoveFail,
                Phase::Run => Phase::Die,
                _ => unreachable!(),
            }
        };

        match m.t {
            MoveType::Reveal => {
                let kaboom = self.board.reveal(m.pos);
                if kaboom { self.phase = lost_phase(&self.phase); }
                if self.phase == Phase::SafeFirstMove { self.phase = Phase::Run }
            },
            MoveType::ToggleFlag => self.board.flag(m.pos),
        };

        if self.phase == Phase::FirstMoveFail {
            let winnable = self.board.mine_count < (self.board.width.get() * self.board.height.get());
            if winnable {
                self.board.hidden_tiles += 1;
                self.board.move_mine_elsewhere(m.pos);
                self.phase = Phase::Run;
                self.act(m);
            } else {
                self.phase = Phase::Die;
            }
        } else if self.phase != Phase::Die && self.board.hidden_tiles == self.board.mine_count {
            self.phase = Phase::Win;
        } else if self.phase == Phase::Die && self.board_conf.reveal_on_lose {
            for tile in self.board.data.iter_mut().filter(|x| is_mine(**x)) {
                *tile = unhide(*tile);
            }
        }
    }
}
impl Board {
    pub fn new(mut conf: BoardConf) -> Self {
        let (w,h) = (conf.w,conf.h);
        let area = w.get()*h.get();
        if w.get() < 3 || h.get() < 3 { conf.revealed_borders = false; }
        let mined_area = area - if conf.revealed_borders { 2*(w.get()-1) + 2*(h.get()-1) } else { 0 };
        let mine_count = ((conf.mine_ratio.0 * mined_area) / conf.mine_ratio.1.get()).clamp(0, mined_area);
        let mut b = Board {
            data: [HIDDEN_BIT].repeat(area),
            width: w,
            height: h,
            hidden_tiles: area,
            mine_count,
            num_tile_reveal: conf.num_tile_reveal,
        };
        if conf.revealed_borders {
            let (w,h) = (w.get(),h.get());
            b.spread_mines(mine_count, true);
            for x in 0..w {
                b.reveal((x,   0).try_into().unwrap());
                b.reveal((x, h-1).try_into().unwrap());
            }
            for y in 1..h-1 {
                b.reveal((  0, y).try_into().unwrap());
                b.reveal((w-1, y).try_into().unwrap());
            }
        } else { b.spread_mines(mine_count, false); }
        b
    }
    pub fn spread_mines(&mut self, mut count: usize, without_edges: bool) {
        let mut rng = thread_rng();
        let w = self.width.get() as u32;
        let h = self.height.get() as u32;
        let (wr,hr) = if without_edges { ((1,w-1),(1,h-1)) } else { ((0,w),(0,h)) };
        while count > 0 {
            let randpos = BoardPos(rng.sample(Uniform::new(wr.0, wr.1)), rng.sample(Uniform::new(hr.0, hr.1)));
            let o = randpos.rel_offset_unchecked(&self);
            if self.data[o] == MINED { continue }
            else {
                self.data[o] = MINED;
                count -= 1;
                self.map_neighs(randpos, |neigh| {
                    if neigh != MINED {
                        neigh + 1
                    } else { neigh }
                });
            }
        }
    }

    fn neighs(&self, pos: BoardPos) -> Vec<BoardPos> {
        const NEIGH_OFFS: &[(isize,isize)] = &[
            (-1,-1),(0,-1),(1,-1),
            (-1, 0),       (1, 0),
            (-1, 1),(0, 1),(1, 1),
        ];
        let ipos: (isize,isize) = pos.try_into().unwrap();
        NEIGH_OFFS
            .iter()
            .filter_map(|(x,y)| (*x + ipos.0, *y + ipos.1).try_into().ok())
            .filter(|pos: &BoardPos| pos.is_within(&self))
            .collect()
    }
    fn map_neighs<F: FnMut(u8) -> u8>(&mut self, pos: BoardPos, mut f: F) {
        let neighs: Vec<usize> = self.neighs(pos).iter().filter_map(|pos| pos.rel_offset(&self)).collect();
        neighs.iter().for_each(|off| { self.data[*off] = f(self.data[*off]); });
    }

    pub fn flood_reveal(&mut self, pos: BoardPos) -> bool {
        let mut queue = vec![pos];
        while let Some(pos) = queue.pop() {
            let off = pos.rel_offset_unchecked(&self);
            let c = &mut self.data[off];
            // don't reveal the already revealed or the flagged, but reveal the questionings
            let unrevealable = (*c & FLAGGED_BIT > 0) ^ (*c & SPECIAL_BIT > 0);
            if *c & HIDDEN_BIT > 0 && !unrevealable {
                *c = unhide(*c);
                self.hidden_tiles -= 1;
                if is_mine(*c) { return true; }
                if *c > 0 { continue; }
                queue.append(&mut self.neighs(pos));
            }
        }
        false
    }
    pub fn reveal_numtile(&mut self, pos: BoardPos) -> bool {
        if let Some(off) = pos.rel_offset(&self) {
            let count = self.data[off] as usize;
            if 1 <= count && count <= 8 {
                let mut neighs = self.neighs(pos);
                let total_neighs = neighs.len();
                neighs.retain(|pos| self.data[pos.rel_offset_unchecked(&self)] & (FLAGGED_BIT | SPECIAL_BIT) != FLAGGED_BIT);
                if (total_neighs - neighs.len()) == count {
                    for pos in neighs.iter() {
                        if self.flood_reveal(*pos) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    pub fn reveal(&mut self, pos: BoardPos) -> bool {
        if let Some(off) = pos.rel_offset(&self) {
            let v = self.data[off];
            if self.num_tile_reveal && 1 <= v && v <= 8 {
                self.reveal_numtile(pos)
            } else {
                self.flood_reveal(pos)
            }
        } else { false }
    }

    pub fn grade(&mut self) {
        for i in &mut self.data {
            if *i == MINED | FLAGGED_BIT {
                *i = CORRECT;
            }
        }
    }
    pub fn flag(&mut self, pos: BoardPos) {
        if let Some(off) = pos.rel_offset(&self) {
            const TOPBIT_MASK: u8 = !(NUMBITS | HIDDEN_BIT);
            let c = &mut self.data[off];
            if *c & HIDDEN_BIT > 0 {
                let new_topbits = match *c & (TOPBIT_MASK) {
                    FLAGGED_BIT => QUESTION,
                    QUESTION => 0,
                    _ => FLAGGED_BIT,
                } | HIDDEN_BIT;
                *c = (*c & NUMBITS) | new_topbits;
            }
        }
    }

    pub fn render(&self) -> Vec<u8> {
        let mut ret = vec![];
        for y in 0..self.height.get() {
            for x in 0..self.width.get() {
                let pos: BoardPos = (x,y).try_into().unwrap();
                let c = &self.data[pos.rel_offset_unchecked(&self)];
                const QUESTION_MASK: u8 = SPECIAL_BIT | FLAGGED_BIT;
                match *c {
                    0 => ret.push(b' '),
                    _ if *c <= 8 => ret.push(b'0' + c),
                    _ if (*c & QUESTION_MASK) == QUESTION_MASK => ret.push(b'Q'),
                    _ if (*c & SPECIAL_BIT) > 0 => ret.push(b'C'),
                    _ if (*c & FLAGGED_BIT) > 0 => ret.push(b'F'),
                    _ if (*c & HIDDEN_BIT) > 0 => ret.push(b'#'),
                    _ if *c == NUMBITS => ret.push(b'O'),
                    _ => ret.push(b'?'),
                }
            }
            ret.extend_from_slice(b"<br>");
        }
        ret
    }

    pub fn move_mine_elsewhere(&mut self, pos: BoardPos) {
        let mut surround_count = 0;
        self.map_neighs(pos, |val| {
            if (val & !FLAGGED_BIT) == MINED {
                surround_count += 1;
                val
            } else {
                val - 1
            }});
        let off = pos.rel_offset_unchecked(&self);
        let vacant_pos = {
            let v = self.data.iter()
                .enumerate()
                .filter(|(_,val)| (*val & NUMBITS) != NUMBITS)
                .map(|(p,_)| p)
                .next()
                .unwrap(); // there must be at least one
            BoardPos((v%self.width.get()) as u32, (v/self.width.get()) as u32)
        };
        let voff = vacant_pos.rel_offset_unchecked(&self);
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

#[derive(Debug, Clone, Copy)]
pub struct BoardPos(u32,u32);
impl BoardPos {
    pub fn rel_offset(&self, b: &Board) -> Option<usize> {
        self.is_within(b).then_some(self.rel_offset_unchecked(b))
    }
    pub fn rel_offset_unchecked(&self, b: &Board) -> usize {
        (self.0 + self.1 * b.width.get() as u32) as usize
    }
    pub fn is_within(&self, b: &Board) -> bool {
        self.0 < b.width.get() as u32 && self.1 < b.height.get() as u32
    }
}
impl TryInto<(isize,isize)> for BoardPos {
    type Error = <usize as TryInto<isize>>::Error;
    fn try_into(self) -> Result<(isize,isize), Self::Error> {
        Ok((self.0.try_into()?, self.1.try_into()?))
    }
}
impl<T: TryInto<u32>> TryFrom<(T,T)> for BoardPos {
    type Error = <T as TryInto<u32>>::Error;
    fn try_from(value: (T,T)) -> Result<Self, Self::Error> {
        Ok(Self(value.0.try_into()?, value.1.try_into()?))
    }
}

pub fn is_mine(v: u8) -> bool {
    (v & NUMBITS) == NUMBITS
}
pub fn unhide(tile: u8) -> u8 {
    tile & NUMBITS
}


