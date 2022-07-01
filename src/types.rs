use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{ AtomicUsize, Ordering },
    },
    fmt::Display,
    ops::{ Deref, DerefMut },
};
use warp::ws::Message;
use tokio::sync::RwLock;
use serde::Serialize;
use crate::minesweeper;
use crate::livepos;

#[derive(Debug, Serialize, Clone)]
pub struct RoomConf {
    pub name: String,
    pub player_cap: usize,
    pub public: bool,
    pub board_conf: minesweeper::BoardConf,
}

pub struct Room {
    pub conf: RoomConf,
    pub players: PlayerMap,
    pub game_driver: tokio::task::JoinHandle<()>,
    pub cmd_stream: CmdTx,
    pub livepos_driver: tokio::task::JoinHandle<()>,
    pub pos_stream: tokio::sync::mpsc::UnboundedSender<livepos::Req>,
}

#[derive(Debug)]
pub enum MetaMove {
    Move(minesweeper::Move,SocketAddr),
    Dump,
    Reset,
}

#[derive(Debug)]
pub struct Conn {
    pub tx: tokio::sync::mpsc::UnboundedSender<Message>,
    pub addr: SocketAddr,
}

#[derive(Debug)]
pub struct Player {
    pub conn: Conn,
    pub uid: usize,
    pub name: String,
    pub clr: String,
}

impl Display for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{}\"@{}", self.name, self.conn.addr)
    }
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, serde::Serialize)]
pub struct RoomId(pub String);
impl Display for RoomId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::borrow::Borrow<str> for RoomId {
    fn borrow(&self) -> &str {
        self.0.borrow()
    }
}

impl RoomId {
    pub fn new_among<'a, I>(existing: I) -> Self
        where
            I: IntoIterator<Item = &'a RoomId>,
            <I as IntoIterator>::IntoIter: Clone,
    {
        use rand::{ thread_rng, Rng, distributions::Alphanumeric };
        let id = RoomId(thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect::<String>());
        let existing = existing.into_iter();
        if existing.clone().any(|x| *x == id) { Self::new_among(existing) }
        else { id }
    }
}

pub type CmdTx = tokio::sync::mpsc::UnboundedSender<MetaMove>;
pub type RoomMap = Arc<RwLock<HashMap<RoomId, Arc<RwLock<Room>>>>>;
pub type PlayerMapData = Arc<RwLock<HashMap<SocketAddr, Player>>>;
#[derive(Debug)]
pub struct PlayerMap {
    inner: PlayerMapData,
    uid_counter: AtomicUsize,
}

impl Deref for PlayerMap {
    type Target = Arc<RwLock<HashMap<SocketAddr, Player>>>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for PlayerMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl Default for PlayerMap {
    fn default() -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())), uid_counter: 0.into() }
    }
}

impl PlayerMap {
    pub async fn insert_conn(&mut self, conn: Conn, name: String, clr: String) -> usize {
        let mut map = self.write().await;
        let uid = self.uid_counter.fetch_add(1, Ordering::Relaxed);
        map.insert(
            conn.addr,
            Player { conn, uid, name, clr },
        );
        uid
    }
}

