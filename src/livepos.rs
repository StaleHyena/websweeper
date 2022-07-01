use crate::types::*;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::Mutex;
use std::collections::{HashMap,HashSet};
use tokio::time::{self, Duration};
use warp::ws::Message;

pub enum ReqData {
    Pos((usize,usize)),
    StateDump,
    Quit,
}

pub struct Req {
    pub id: usize,
    pub data: ReqData,
}

pub async fn livepos(players: PlayerMapData, mut recv: tokio_mpsc::UnboundedReceiver<Req>) {
    let positions = Mutex::new(HashMap::new());
    let dirty = Mutex::new(HashSet::new());
    let process_upds = async {
        while let Some(update) = recv.recv().await {
            let mut dirty = dirty.lock().await;
            let mut positions = positions.lock().await;
            match update.data {
                ReqData::Pos(p) => {
                    let old = positions.get(&update.id).unwrap_or(&(0,0));
                    if p != *old {
                        dirty.insert(update.id);
                    }
                    positions.insert(update.id, p);
                },
                ReqData::StateDump => {
                    dirty.clear();
                    dirty.extend(positions.keys().copied());
                },
                ReqData::Quit => {
                    positions.remove(&update.id);
                    dirty.retain(|x| *x != update.id);
                }
            }
        }
    };
    let periodic_send = async {
        let mut interv = tokio::time::interval(Duration::from_millis(16));
        interv.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        loop {
            interv.tick().await;
            let mut dirty = dirty.lock().await;
            if dirty.len() > 0 {
                let mut positions = positions.lock().await;
                let msg = jsonenc_ids(&mut positions, &*dirty).expect("couldn't JSONify player positions");
                dirty.clear();
                let plock = players.read().await;
                for player in plock.values() {
                    if let Err(e) = player.conn.tx.send(Message::text(format!("pos {}", msg))) {
                        println!("E: couldn't send livepos update to {}: {}", player, e);
                    }
                }
            }
        }
    };

    tokio::select!(
        _ = process_upds => (),
        _ = periodic_send => ()
    );
}

fn jsonenc_ids<'a, I: IntoIterator<Item=&'a usize>>(positions: &mut HashMap<usize, (usize,usize)>, ids: I) -> Result<String, serde_json::Error> {
    let mut pairs = Vec::new();
    for id in ids {
        pairs.push((id, positions[id]));
    };
    serde_json::to_string(&pairs)
}

