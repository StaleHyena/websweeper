use std::{
    error::Error,
    net::SocketAddr,
    sync::Arc,
    collections::HashMap,
    num::NonZeroUsize,
};
use futures::stream::StreamExt;

mod types;
mod livepos;
mod conn;
mod minesweeper;
use types::*;

use tokio::sync::RwLock;

const FONT_FILE: &[u8] = include_bytes!("../assets/VT323-Regular.ttf");
const CONF_FILE: &str = "./conf.json";

fn main() -> Result<(), Box<dyn Error>> {
    let conf = Config {
        cert: "./cert.pem".to_owned(),
        pkey: "./cert.rsa".to_owned(),
        index_pg: "./assets/index.html".to_owned(),
        room_pg: "./assets/room.html".to_owned(),
        client_code: "./assets/client.js".to_owned(),
        stylesheet: "./assets/style.css".to_owned(),
        socket_addr: ([0,0,0,0],31235).into(),
    };

    tokio_main(conf)
}

#[tokio::main]
async fn tokio_main(conf: Config) -> Result<(), Box<dyn Error>> {
    let conf_file: serde_json::Value = serde_json::from_str(&tokio::fs::read_to_string(CONF_FILE).await?)?;
    let area_limit: usize = conf_file.get("area_limit")
        .expect("no area_limit field in the conf.json file")
        .as_u64().expect("area_limit not a number") as usize;
    let room_limit: usize = conf_file.get("room_limit")
        .expect("no room_limit field in the conf.json file")
        .as_u64().expect("room_limit not a number") as usize;
    let rooms: RoomMap = Arc::new(RwLock::new(HashMap::new()));
    let public_rooms = Arc::new(RwLock::new(HashMap::new()));
    use warp::*;

    let index = path::end().and(fs::file(conf.index_pg.clone()));
    let style = path!("s.css").and(fs::file(conf.stylesheet.clone()));
    let code = path!("c.js").and(fs::file(conf.client_code.clone()));
    let font = path!("f.ttf").map(|| FONT_FILE);
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

        path!("rspace").and_then(move || {
            let r = rooms.clone();
            async move {
                let empty_len = empty_rooms(r.clone()).await.len();
                let space = room_limit - r.read().await.len() + empty_len;
                Ok::<_,std::convert::Infallible>(
                    hyper::Response::builder()
                        .status(hyper::StatusCode::OK)
                        .body(hyper::Body::from(space.to_string()))
                        .unwrap()
                )
            }
        })
    };
    let rform_recv = {
        let rooms = rooms.clone();
        let pubs = public_rooms.clone();
        post().and(path("r")).and(body::content_length_limit(4096)).and(body::form())
        .and_then(move |rinfo: HashMap<String, String>| {
            println!("{:?}", rinfo);
            let rooms = rooms.clone();
            let pubs = pubs.clone();
            async move {
                let slots_available = room_limit - rooms.read().await.len();
                let empty = empty_rooms(rooms.clone()).await;
                if slots_available < 1 {
                    if slots_available + empty.len() > 0 {
                        remove_room(rooms.clone(), pubs.clone(), empty[0].clone()).await;
                    } else {
                        return Err(reject::custom(NoRoomSlots));
                    }
                }

                if let (Some(w),Some(h),Some(num),Some(denom),access,asfm,limit) = (
                    rinfo.get("rwidth").and_then(|wt| wt.parse::<NonZeroUsize>().ok()),
                    rinfo.get("rheight").and_then(|ht| ht.parse::<NonZeroUsize>().ok()),
                    rinfo.get("rration").and_then(|nt| nt.parse::<usize>().ok()),
                    rinfo.get("rratiod").and_then(|dt| dt.parse::<NonZeroUsize>().ok()),
                    rinfo.get("raccess"),
                    rinfo.get("ralwayssafe1move"),
                    rinfo.get("rlimit").and_then(|l| l.parse::<usize>().ok()),
                    ) {
                    if w.get()*h.get() > area_limit {
                        return Err(reject::custom(BoardTooBig))
                    }
                    let board_conf = minesweeper::BoardConf { w, h, mine_ratio: (num,denom), always_safe_first_move: asfm.is_some() };
                    let mut rooms = rooms.write().await;
                    let uid = types::RoomId::new_in(&rooms);
                    let name = {
                        let n = rinfo.get("rname").unwrap().to_owned();
                        if n.is_empty() { uid.to_string() } else { n }
                    };

                    let players = PlayerMap::default();

                    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
                    let game_handle = tokio::spawn(gameloop(cmd_rx, players.clone(), board_conf));

                    let (pos_tx, pos_rx) = tokio::sync::mpsc::unbounded_channel();
                    let livepos_handle = tokio::spawn(livepos::livepos(players.clone(), pos_rx));

                    let room_conf = RoomConf {
                        name,
                        player_cap: match limit { Some(i) => i, None => usize::MAX },
                        public: access.is_some(),
                        board_conf,
                    };
                    let new_room = Room {
                        conf: room_conf,
                        players,
                        game_driver: game_handle,
                        cmd_stream: cmd_tx,
                        livepos_driver: livepos_handle,
                        pos_stream: pos_tx,
                    };
                    if access.is_some() {
                        pubs.write().await.insert(uid.clone(), serde_json::to_string(&new_room.conf).unwrap());
                    }
                    rooms.insert(uid.clone(), Arc::new(RwLock::new(new_room)));

                    Ok(
                    hyper::Response::builder()
                       .status(hyper::StatusCode::SEE_OTHER)
                       .header(hyper::header::LOCATION, format!("./room/{uid}"))
                       .body(hyper::Body::empty())
                       .unwrap()
                    )
                } else { Err(reject::custom(BadFormData)) }
            }
        })
    };
    let room = {
        let rooms_ws = rooms.clone();
        let rooms_lobby = rooms.clone();
        let prefix = get().and(path!("room" / String / ..));

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
                                conn::lobby(socket, saddr.expect("socket without address"), (id,r))
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
                .and(fs::file(conf.room_pg.clone()))
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
        .or(style)
        .or(code)
        .or(font)
        .or(listing)
        .or(roomspace)
        .or(rform_recv)
        .or(room)
        .recover(error_handler);

    let server = warp::serve(route)
        .tls()
        .cert_path(conf.cert)
        .key_path(conf.pkey)
        .run(conf.socket_addr);
    println!("Serving on {}", conf.socket_addr);
    server.await;
    Ok(())
}

// If a move is made, broadcast new board, else just send current board
async fn gameloop(mut move_rx: tokio::sync::mpsc::UnboundedReceiver<MetaMove>, players: PlayerMapData, bconf: minesweeper::BoardConf) {
    use minesweeper::*;
    use flate2::{ Compression, write::DeflateEncoder };
    use std::io::Write;
    let mut game = Game::new(bconf);
    let mut latest_player_name = None;
    while let Some(req) = move_rx.recv().await {
        let done = game.phase == Phase::Die || game.phase == Phase::Win;
        match req {
            MetaMove::Move(m, o) => if !done {
                game = game.act(m);
                if game.phase == Phase::Win || game.phase == Phase::Die {
                    game.board = game.board.grade();
                }
                latest_player_name = players.read().await.get(&o).map(|p| p.name.clone());
            },
            MetaMove::Dump => (),
            MetaMove::Reset => { game = Game::new(bconf); },
        }
        use warp::ws::Message;
        let mut board_encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        board_encoder.write_all(&game.board.render()).unwrap();
        let compressed_board = board_encoder.finish().unwrap();
        let mut reply = vec![Message::binary(compressed_board)];
        let lpname = latest_player_name.as_deref().unwrap_or("unknown player").replace(' ', "&nbsp");
        match game.phase {
            Phase::Win => { reply.push(Message::text(format!("win {lpname}"))); },
            Phase::Die => { reply.push(Message::text(format!("lose {lpname}"))); },
            _ => (),
        }
        {
            let peers = players.read().await;
            for (addr, p) in peers.iter() {
                for r in reply.iter() {
                    if let Err(e) = p.conn.tx.send(r.clone()) {
                        println!("couldn't send game update {r:?} to {addr}: {e}");
                    }
                }
            }
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

async fn empty_rooms(rooms: RoomMap) -> Vec<RoomId> {
    let rl = rooms.read().await;
    futures::stream::iter(rl.iter())
        .filter_map(|(id,roomarc)| async move {
            let rrl = roomarc.read().await;
            let rrrl = rrl.players.read().await;
            if rrrl.len() == 0 { Some(id.clone()) } else { None }
        })
        .collect::<Vec<RoomId>>().await
}

async fn remove_room<T>(rooms: RoomMap, pubs: Arc<RwLock<HashMap<RoomId,T>>>, id: RoomId) {
    {
        let mut rwl = rooms.write().await;
        rwl.remove(&id);
    }
    {
        let mut pwl = pubs.write().await;
        pwl.remove(&id);
    }
}

