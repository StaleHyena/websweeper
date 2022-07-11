use std::{
    error::Error,
    net::SocketAddr,
    sync::Arc,
    collections::HashMap,
    num::NonZeroUsize,
    path::PathBuf,
};
use futures::stream::StreamExt;
use tokio::sync::RwLock;
use serde::Deserialize;

mod types;
mod livepos;
mod conn;
mod minesweeper;
use types::*;

const CONF_FILE: &str = "./conf.json";

#[derive(Deserialize)]
struct ConfPaths {
    pub cert: PathBuf,
    pub pkey: PathBuf,
    pub assets: PathBuf,
    pub index_page: PathBuf,
    pub room_page: PathBuf,
}
#[derive(Deserialize)]
struct ConfServer {
    pub listen_on: SocketAddr,
}
#[derive(Deserialize)]
struct ConfLimits {
    pub board_area: usize,
    pub room_slots: usize,
    pub form_size: u64,
    pub inbound_packet_size: usize,
}
#[derive(Deserialize)]
struct Conf {
    pub paths: ConfPaths,
    pub server: ConfServer,
    pub limits: ConfLimits,
}

fn main() -> Result<(), Box<dyn Error>> {
    let conf: Conf = serde_json::from_str(&std::fs::read_to_string(CONF_FILE)?)?;
    tokio_main(conf)
}

#[tokio::main]
async fn tokio_main(conf: Conf) -> Result<(), Box<dyn Error>> {
    let conf = Arc::new(conf);
    let rooms = Arc::new(RwLock::new(RoomMap::new()));
    let public_rooms = Arc::new(RwLock::new(HashMap::new()));
    use warp::*;

    let index = path::end().and(fs::file(conf.paths.index_page.clone()));
    let assets = any().and(fs::dir(conf.paths.assets.clone()));
    let listing = {
        let rooms = rooms.clone();
        let pubs = public_rooms.clone();

        path!("rlist").and_then(move || {
            let rooms = rooms.clone();
            let pubs = pubs.clone();
            async move {
                let roomsl = rooms.read().await;
                let pubsl = pubs.read().await;
                let rooms_pcount = futures::stream::iter(pubsl.iter())
                    .then(|(id, _):(&RoomId,_)| {
                        let roomsl = roomsl.clone();
                        async move {
                            let room = roomsl.get(id).unwrap().read().await;
                            let pcount = room.players.read().await.len();
                            (id.clone(), (pcount, room.conf.player_cap))
                        }
                    })
                    .collect::<HashMap<RoomId,_>>().await;
                let resp = (&*pubsl, rooms_pcount);
                Ok::<_,std::convert::Infallible>(
                    reply::json(&resp)
                )
            }
        })
    };
    let roomspace = {
        let rooms = rooms.clone();
        let conf = conf.clone();

        path!("rspace").and_then(move || {
            let r = rooms.clone();
            let conf = conf.clone();
            async move {
                let r = r.read().await;
                let empty_len = empty_rooms(&r).await.len();
                let space = conf.limits.room_slots - r.len() + empty_len;
                Ok::<String, std::convert::Infallible>(space.to_string())
            }
        })
    };
    let rform_recv = {
        let rooms = rooms.clone();
        let pubs = public_rooms.clone();
        let conf = conf.clone();

        post().and(path("r")).and(body::content_length_limit(conf.limits.form_size)).and(body::form())
        .and_then(move |rinfo: HashMap<String, String>| {
            let rooms = rooms.clone();
            let pubs = pubs.clone();
            let conf = conf.clone();
            async move {
                let slots_available = conf.limits.room_slots - rooms.read().await.len();
                let empty = empty_rooms(&*rooms.read().await).await;
                if slots_available < 1 {
                    if slots_available + empty.len() > 0 {
                        let mut roomsl = rooms.write().await;
                        let mut pubsl = pubs.write().await;
                        remove_room(&mut *roomsl, &mut *pubsl, empty[0].clone());
                    } else {
                        return Err(reject::custom(NoRoomSlots));
                    }
                }

                let mut rooms = rooms.write().await;
                let uid = RoomId::new_among(rooms.keys());

                match room_from_form(uid.clone(), &rinfo, &conf) {
                    Ok((room, public)) => {
                        if public {
                            pubs.write().await.insert(uid.clone(), serde_json::to_string(&room.conf).unwrap());
                            println!("New public room: {:?}", room.conf);
                        } else {
                            println!("New private room: {:?}", room.conf);
                        }
                        rooms.insert(uid.clone(), Arc::new(RwLock::new(room)));

                        Ok(
                            hyper::Response::builder()
                            .status(hyper::StatusCode::SEE_OTHER)
                            .header(hyper::header::LOCATION, format!("./room/{uid}"))
                            .body(hyper::Body::empty())
                            .unwrap()
                          )
                    },
                    Err(e) => Err(e),
                }
            }
        })
    };
    let room = {
        let rooms_ws = rooms.clone();
        let rooms_lobby = rooms.clone();
        let prefix = get().and(path!("room" / String / ..));
        let max_inbound_packet_size = conf.limits.inbound_packet_size;
        let room_path = conf.paths.room_page.clone();

        // Fixme: better errors
        prefix.and(path!("ws"))
            .and(ws())
            .and(addr::remote())
            .and_then(move |id: String, websocket: warp::ws::Ws, saddr: Option<SocketAddr>| {
                let rooms = rooms_ws.clone();
                async move {
                    let id = RoomId(id);
                    match rooms.read().await.get(&id).cloned() {
                        Some(r) => {
                            println!("{id} I: conn from {saddr:?}");
                            Ok(websocket.on_upgrade(move |socket| {
                                conn::setup_conn(socket, saddr.expect("socket without address"), (id,r), max_inbound_packet_size)
                            }))
                        },
                        None => {
                            println!("I: conn from {saddr:?} into inexistent room {id}");
                            Err(reject())
                        }
                    }
                }
            })
            .or(prefix.and(path::end())
                .and(fs::file(room_path))
                .then(move |id: String, f: fs::File| {
                    let rooms = rooms_lobby.clone();
                    async move {
                        if rooms.read().await.contains_key(&RoomId(id)) {
                            f.into_response()
                        } else {
                            reply::with_status("No such room", http::StatusCode::BAD_REQUEST).into_response()
                        }
                    }
                })
            )
    };


    let route = get()
        .and(index)
        .or(listing)
        .or(roomspace)
        .or(rform_recv)
        .or(room)
        .or(assets)
        .recover(error_handler);

    let server = warp::serve(route)
        .tls()
        .cert_path(conf.paths.cert.clone())
        .key_path(conf.paths.pkey.clone())
        .run(conf.server.listen_on);
    println!("Serving on {}", conf.server.listen_on);
    server.await;
    Ok(())
}

// If a move is made, broadcast new board, else just send current board
type MoveStreamHandles = (tokio::sync::mpsc::UnboundedSender<MetaMove>, tokio::sync::mpsc::UnboundedReceiver<MetaMove>);
async fn gameloop(moves: MoveStreamHandles, players: Arc<RwLock<PlayerMap>>, bconf: minesweeper::BoardConf) {
    // FIXME: push new board if and only if there aren't any remaining commands in the queue
    use minesweeper::*;
    use flate2::{ Compression, write::DeflateEncoder };
    use std::io::Write;
    let (move_tx, mut move_rx) = moves;
    let mut game = Game::new(bconf);
    let mut final_player_name = None;
    let mut desynced = true;
    while let Some(req) = move_rx.recv().await {
        let done = |p: &Phase| { *p == Phase::Die || *p == Phase::Win };
        match req {
            MetaMove::Move(m, o) => if !done(&game.phase) {
                game = game.act(m);
                desynced = true;
                if done(&game.phase) {
                    game.board = game.board.grade();
                    final_player_name = players.read().await.get(&o).map(|p| p.name.clone());
                }
                move_tx.send(MetaMove::StateSync).unwrap();
            },
            MetaMove::StateSync => { // a StateDump, but consecutive ones in the queue get merged
                if desynced { move_tx.send(MetaMove::StateDump).unwrap(); desynced = false; }
            },
            MetaMove::StateDump => {
                use warp::ws::Message;
                let mut board_encoder = DeflateEncoder::new(Vec::new(), Compression::default());
                board_encoder.write_all(&game.board.render()).unwrap();
                let compressed_board = board_encoder.finish().unwrap();
                let mut reply = vec![Message::binary(compressed_board)];
                let lpname = final_player_name.as_deref().unwrap_or("unknown player").replace(' ', "&nbsp");
                match game.phase {
                    Phase::Win => { reply.push(Message::text(format!("win {lpname}"))); },
                    Phase::Die => { reply.push(Message::text(format!("lose {lpname}"))); },
                    _ => (),
                }
                let peers = players.read().await;
                for (addr, p) in peers.iter() {
                    for r in reply.iter() {
                        if let Err(e) = p.conn.tx.send(r.clone()) {
                            println!("couldn't send game update {r:?} to {addr}: {e}");
                        }
                    }
                }
                desynced = false;
            },
            MetaMove::Reset => {
                if done(&game.phase) {
                    game = Game::new(bconf);
                    move_tx.send(MetaMove::StateDump).unwrap();
                }
            },
        }
    }
}

use warp::{ reject::{ Reject, Rejection }, reply::{ self, Reply }, http::StatusCode };
#[derive(Debug)]
struct BadFormData;
impl Reject for BadFormData {}

#[derive(Debug)]
struct BoardTooBig;
impl Reject for BoardTooBig {}

#[derive(Debug)]
struct NoRoomSlots;
impl Reject for NoRoomSlots {}

async fn error_handler(err: Rejection) -> Result<impl Reply, std::convert::Infallible> {
    if err.is_not_found() { Ok(reply::with_status("No such file", StatusCode::NOT_FOUND)) }
    else if let Some(_e) = err.find::<BadFormData>() {
        Ok(reply::with_status("Bad form data", StatusCode::BAD_REQUEST))
    } else if let Some(_e) = err.find::<BoardTooBig>() {
        Ok(reply::with_status("Board too big", StatusCode::BAD_REQUEST))
    } else if let Some(_e) = err.find::<NoRoomSlots>() {
        Ok(reply::with_status("No more rooms slots", StatusCode::BAD_REQUEST))
    } else {
        println!("unhandled rejection: {err:?}");
        Ok(reply::with_status("Server error", StatusCode::INTERNAL_SERVER_ERROR))
    }
}

async fn empty_rooms(rooms: &RoomMap) -> Vec<RoomId> {
    futures::stream::iter(rooms.iter())
        .filter_map(|(id,roomarc)| async move {
            let rrl = roomarc.read().await;
            let rrrl = rrl.players.read().await;
            if rrrl.len() == 0 { Some(id.clone()) } else { None }
        })
        .collect::<Vec<RoomId>>().await
}

fn room_from_form(uid: RoomId, rinfo: &HashMap<String,String>, conf: &Conf) -> Result<(types::Room, bool), Rejection> {
    if let (Some(w),Some(h),Some(num),Some(denom),public,asfm,rborders,revealol,Some(limit)) = (
        rinfo.get("bwidth").and_then(|w| w.parse::<NonZeroUsize>().ok()),
        rinfo.get("bheight").and_then(|h| h.parse::<NonZeroUsize>().ok()),
        rinfo.get("mineratio-n").and_then(|n| n.parse::<usize>().ok()),
        rinfo.get("mineratio-d").and_then(|d| d.parse::<NonZeroUsize>().ok()),
        rinfo.get("public").map(|s| s == "on").unwrap_or(false),
        rinfo.get("allsafe1move").map(|s| s == "on").unwrap_or(false),
        rinfo.get("rborders").map(|s| s == "on").unwrap_or(false),
        rinfo.get("revealonlose").map(|s| s == "on").unwrap_or(false),
        rinfo.get("limit").and_then(|l| l.parse::<NonZeroUsize>().ok()),
        ) {
        if w.get()*h.get() > conf.limits.board_area {
            return Err(warp::reject::custom(BoardTooBig))
        }
        let board_conf = minesweeper::BoardConf {
            w, h, mine_ratio: (num,denom),
            always_safe_first_move: asfm, revealed_borders: rborders, reveal_on_lose: revealol
        };
        let name = {
            let n = rinfo.get("rname").unwrap().to_owned();
            if n.is_empty() { uid.to_string() } else { n }
        };

        let players = Arc::new(RwLock::new(PlayerMap::default()));

        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let game_handle = tokio::spawn(gameloop((cmd_tx.clone(), cmd_rx), players.clone(), board_conf));

        let (pos_tx, pos_rx) = tokio::sync::mpsc::unbounded_channel();
        let livepos_handle = tokio::spawn(livepos::livepos(players.clone(), pos_rx));

        let room_conf = RoomConf {
            name,
            player_cap: limit,
            public,
            board_conf,
        };
        Ok((Room {
            conf: room_conf,
            players,
            game_driver: game_handle,
            cmd_stream: cmd_tx,
            livepos_driver: livepos_handle,
            pos_stream: pos_tx,
        }, public))
    } else { Err(warp::reject::custom(BadFormData)) }
}

fn remove_room<T>(rooms: &mut RoomMap, pubs: &mut HashMap<RoomId,T>, id: RoomId) {
    rooms.remove(&id);
    pubs.remove(&id);
}

