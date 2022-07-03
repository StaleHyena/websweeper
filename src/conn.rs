use crate::types::*;
use std::{
    sync::Arc,
    net::SocketAddr,
};
use tokio::sync::RwLock;
use futures::{SinkExt, TryStreamExt, StreamExt, stream::SplitStream};
use warp::ws::{ WebSocket, Message };
use crate::livepos;

pub async fn setup_conn(socket: WebSocket, addr: SocketAddr, rinfo: (RoomId,Arc<RwLock<Room>>), max_in: usize) {
    let (room_id, room) = rinfo;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (mut outgoing, incoming) = socket.split();
    let conn = Conn { addr, tx };

    println!("{room_id} I: Incoming TCP connection from: {}", addr);

    let full = {
        let rl = room.read().await;
        let pcap = rl.conf.player_cap;
        let pl = rl.players.read().await;
        pl.len() >= pcap.get()
    };
    if full { return }
    let drive_game = drive_conn((conn, incoming), (room_id.clone(),room.clone()), max_in);
    let send_to_client = {
        let room_id = room_id.clone();
        async move {
            while let Some(m) = rx.recv().await {
                if let Err(e) = outgoing.send(m).await {
                    println!("{room_id} E: something went bad lol: {e}");
                }
            }
        }
    };

    tokio::select! {
        _ = drive_game => (),
        _ = send_to_client => { println!("{room_id} E: anomalous close for {addr}"); }
    };

    let room_lock = room.read().await;
    let mut players = room_lock.players.write().await;
    if let Some(disconn_p) = players.remove(&addr) {
        if let Err(e) = room_lock.pos_stream.send(livepos::Req { id: disconn_p.uid, data: livepos::ReqData::Quit }) {
            println!("{room_id} E: couldn't send removal request for {disconn_p} from the live position system: {e}");
        }
        for p in players.values() {
            if let Err(e) = p.conn.tx.send(Message::text(format!("logoff {}", disconn_p.uid))) {
                println!("{room_id} E: couldn't deliver logoff info to {}: {}", p, e);
            }
        }
        println!("{room_id} I: {disconn_p} disconnected");
    } else {
        println!("{room_id} I: {addr} disconnected");
    }
}


pub async fn drive_conn(conn: (Conn, SplitStream<WebSocket>), rinfo: (RoomId, Arc<RwLock<Room>>), max_in: usize) {
    let (conn, mut incoming) = conn;
    let (room_id, room) = rinfo;
    let (players, cmd_tx, pos_tx, room_conf) = {
        let room = room.read().await;
        (room.players.clone(), room.cmd_stream.clone(), room.pos_stream.clone(), room.conf.clone())
    };
    while let Ok(cmd) = incoming.try_next().await {
        if let Some(cmd) = cmd {
            // if it ain't text we can't handle it
            let cmd = match cmd.to_str() {
                Ok(cmd) => { if cmd.len() > max_in {
                    println!("{room_id} E: string too big: {cmd}");
                    return
                } else { cmd.to_owned() } },
                Err(_) => return
            };

            let mut fields = cmd.split(" ");
            let parse_pos = |mut fields: std::str::Split<&str>| -> Option<(usize, usize)> {
                let x = fields.next().and_then(|xstr| xstr.parse::<usize>().ok());
                let y = fields.next().and_then(|ystr| ystr.parse::<usize>().ok());
                x.zip(y)
            };
            if let Some(cmd_name) = fields.next() {
                if cmd_name == "<3" {
                    continue; // heartbeat, no need to handle it
                }
                use crate::minesweeper::{Move,MoveType};
                let mut players_lock = players.write().await;
                match players_lock.get_mut(&conn.addr) {
                    Some(me) => match cmd_name {
                        "pos" => {
                            if let Some(pos) = parse_pos(fields) {
                                if let Err(e) = pos_tx.send(livepos::Req { id: me.uid, data: livepos::ReqData::Pos(pos) }) {
                                    println!("{room_id} E: couldn't process {me}'s position update: {e}");
                                };
                            }
                        },
                        "reveal" => {
                            match parse_pos(fields) {
                                Some(pos) => {
                                    if let Err(e) = cmd_tx.send(MetaMove::Move(Move { t: MoveType::Reveal, pos }, conn.addr)) {
                                        println!("{room_id} E: couldn't process {me}'s reveal command: {e}");
                                    };
                                },
                                None => {
                                    println!("{room_id} E: bad reveal from {me}");
                                }
                            }
                        },
                        "flag" => {
                            match parse_pos(fields) {
                                Some(pos) => {
                                    if let Err(e) = cmd_tx.send(MetaMove::Move(Move { t: MoveType::ToggleFlag, pos }, conn.addr)) {
                                        println!("{room_id} E: couldn't process {me}'s flag command: {e}");
                                    };
                                },
                                None => {
                                    println!("{room_id} E: bad flag from {me}");
                                }
                            }
                        },
                        "reset" => {
                            if let Err(e) = cmd_tx.send(MetaMove::Reset) {
                                println!("{room_id} E: couldn't request game dump in behalf of {me}: {e}");
                            }
                        },
                        e => println!("{room_id} E: unknown command {e:?} from {me}: \"{cmd}\""),
                    },
                    None => {
                        if cmd_name == "register" {
                            let mut all_fields = fields.collect::<Vec<&str>>();
                            let clr = all_fields.pop().expect("register without color").chars().filter(|c| c.is_digit(16) || *c == '#').collect::<String>();
                            let name = {
                                let def = "anon".to_string();
                                if all_fields.is_empty() { def }
                                else {
                                    let n = ammonia::clean(&all_fields.join(" "));
                                    if n.is_empty() { def } else { n }
                                }
                            };
                            println!("{room_id} I: registered \"{name}@{}\"", conn.addr);
                            drop(players_lock);
                            let uid = {
                                // new scope cuz paranoid bout deadlocks
                                room.write().await.players.write().await.insert_conn(conn.clone(), name.clone(), clr)
                            };
                            let players_lock = players.read().await;
                            let me = players_lock.get(&conn.addr).unwrap();
                            conn.tx.send(Message::text(format!("regack {} {} {} {}",
                                    room_conf.name.replace(' ', "&nbsp;"), name.replace(' ', "&nbsp;"), uid, room_conf.board_conf))
                            ).expect("couldn't send register ack");

                            {
                                let msg = Message::text(format!("players {}",
                                            jsonenc_players(players_lock.values())
                                            .expect("couldn't JSONify players")));
                                for p in players_lock.values() {
                                    if let Err(e) = p.conn.tx.send(msg.clone()) {
                                        println!("{room_id} E: couldn't dump players for {me}: {e}");
                                    }
                                }
                            }
                            if let Err(e) = pos_tx.send(livepos::Req { id: uid, data: livepos::ReqData::StateDump }) {
                                println!("{room_id} E: couldn't request position dump for {me}: {e}");
                            }
                            if let Err(e) = cmd_tx.send(MetaMove::Dump) {
                                println!("{room_id} E: couldn't request game dump for {me}: {e}");
                            }
                        }
                    }
                }
            }
        } else {
            println!("{room_id} E: reached end of stream for {}", conn.addr);
            break;
        }
    }
}

fn jsonenc_players<'a, I: IntoIterator<Item=&'a Player>>(players: I) -> Result<String, serde_json::Error> {
    let mut pairs = Vec::new();
    for player in players {
        pairs.push((player.uid, player.name.replace(' ', "&nbsp"), player.clr.clone()));
    }
    serde_json::to_string(&pairs)
}
