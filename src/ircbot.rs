//use irc::client::prelude::*;
use crate::types::{RoomConf, CmdTx};
use tokio::sync::mpsc as tokio_mpsc;
use serde::Deserialize;
//use futures::prelude::*;

#[derive(Debug)]
pub enum IrcCmd {
    NameTakenQuery(String, tokio::sync::oneshot::Sender<bool>),
    GameWin(String),
    GameLose(String),
}

pub type IrcCmdTx = tokio_mpsc::UnboundedSender<IrcCmd>;

#[derive(Deserialize, Clone)]
pub struct IrcConf {
    pub server: String,
    pub port: u16,
}

pub async fn manage_irc_channel(_irc_conf: IrcConf, _room_conf: RoomConf, _game_tx: CmdTx, mut irc_rx: tokio_mpsc::UnboundedReceiver<IrcCmd>) {
    // turns out none of the irc libs i tried worked and i lost interest
    //
    // let channel_name = format!("#mines-{}", room_conf.name);
    // let bot_name = format!("mines-bot-{}", room_conf.name);
    // let config = Config {
    //     nickname: Some(bot_name.clone()),
    //     username: Some(bot_name.clone()),
    //     realname: Some(bot_name.clone()),
    //     server: Some(irc_conf.server),
    //     port: Some(irc_conf.port),
    //     encoding: Some("UTF-8".to_string()),
    //     channels: vec![channel_name.clone()],
    //     umodes: Some("+B-x".to_string()),
    //     user_info: Some("websweeper channel manager bot".to_string()),
    //     use_tls: Some(true),
    //     ping_time: Some(20),
    //     ping_timeout: Some(15),
    //     ..Default::default()
    // };

    // let mut client = Client::from_config(config).await.expect("couldn't create an irc client");
    // client.identify().expect("couldn't identify irc bot");

    // println!("irc bot {:#?}", client);

    // if !room_conf.public {
    //     client.send_mode(&channel_name, &[Mode::Plus(ChannelMode::Secret, None)]).expect("couldn't set irc channel mode");
    // }
    // client.send_mode(&channel_name,
    //     &[Mode::Plus(ChannelMode::Limit, Some(room_conf.player_cap.to_string()))]
    // ).expect("couldn't set irc channel mode");

    while let Some(req) = irc_rx.recv().await {
        match req {
            IrcCmd::NameTakenQuery(_nick, res_tx) => {
                // let taken: bool = client.list_users(&channel_name)
                //     .and_then(|userlist| {
                //         userlist.iter().position(|u| u.get_nickname() == nick)
                //     }).is_some();
                // res_tx.send(taken).unwrap();
                res_tx.send(false).unwrap();
            },
            IrcCmd::GameWin(_nick) => {
                // println!("irc {nick} win");
                // if let Err(e) = client.send(Command::PRIVMSG(channel_name.clone(), format!("You win! {nick} made the winning move."))) {
                //     println!("couldn't send irc win message: {e}");
                // }
            },
            IrcCmd::GameLose(_nick) => {
                // println!("irc {nick} lose");
                // if let Err(e) = client.send(Command::PRIVMSG(channel_name.clone(), format!("You win! {nick} made the winning move."))) {
                //     println!("couldn't send irc lose message: {e}");
                // }
            },
        }
    }
}
