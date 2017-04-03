#[macro_use] extern crate log;
extern crate ws;
extern crate env_logger;
extern crate serde;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;

use std::fs::OpenOptions;
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::HashSet;

use serde_json::Value as Json;

const ADDR: &'static str = "127.0.0.1:3012";
const SAVE: ws::util::Token = ws::util::Token(1);
const PING: ws::util::Token = ws::util::Token(2);
const FILE: &'static str = "message_log";
const SAVE_TIME: u64 = 500;
const PING_TIME: u64 = 10_000;

type MessageLog = Rc<RefCell<Vec<Message>>>;
type Users = Rc<RefCell<HashSet<String>>>;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Wrapper {
    path: String,
    content: Json,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Message {
    nick: String,
    message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Join {
    join_nick: String,
}

struct ChatHandler {
    out: ws::Sender,
    nick: Option<String>,
    message_log: MessageLog,
    users: Users,
}

impl ws::Handler for ChatHandler {

    fn on_open(&mut self, _: ws::Handshake) -> ws::Result<()> {
        try!(self.out.timeout(SAVE_TIME, SAVE));
        try!(self.out.timeout(PING_TIME, PING));
        let backlog = self.message_log.borrow();
        // We take two chunks because one chunk might not be a full 50
        let mut it = backlog.chunks(50).rev().take(2);

        let msgs1 = it.next();
        let msgs2 = it.next();

        // longwinded reverse
        if let Some(msgs) = msgs2 {
            for msg in msgs {

                try!(self.out.send(format!("{:?}", json!({
                    "path": "/message",
                    "content": msg.clone(),
                }))))
            }
        }

        if let Some(msgs) = msgs1 {
            for msg in msgs {
                try!(self.out.send(format!("{:?}", json!({
                    "path": "/message",
                    "content": msg.clone(),
                }))))
            }
        }
        Ok(())
    }

    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        if let Ok(text_msg) = msg.clone().as_text() {
            if let Ok(wrapper) = serde_json::from_str::<Wrapper>(text_msg) {
                if let Ok(simple_msg) = serde_json::from_value::<Message>(wrapper.content.clone()) {
                    self.message_log.borrow_mut().push(simple_msg);
                    return self.out.broadcast(msg)
                }

                if let Ok(join) = serde_json::from_value::<Join>(wrapper.content.clone()) {
                    if self.users.borrow().contains(&join.join_nick) {
                        return self.out.send(format!("{:?}", json!({
                            "path": "/error",
                            "content": "A user by that name already exists.",
                        })))
                    }

                    let join_msg = Message {
                        nick: "system".into(),
                        message: format!("{} has joined the chat.", join.join_nick),
                    };
                    self.users.borrow_mut().insert(join.join_nick.clone());
                    self.nick = Some(join.join_nick);
                    self.message_log.borrow_mut().push(join_msg.clone());
                    return self.out.broadcast(format!("{:?}", json!({
                        "path": "/joined",
                        "content": join_msg,
                    })))
                }
            }
        }
        self.out.send(format!("{:?}", json!({
            "path": "/error",
            "content": format!("Unable to parse message {:?}", msg),
        })))
    }

    fn on_close(&mut self, _: ws::CloseCode, _: &str) {
        if let Some(nick) = self.nick.as_ref() {
            self.users.borrow_mut().remove(nick);
            let leave_msg = Message {
                nick: "system".into(),
                message: format!("{} has left the chat.", nick),
            };
            self.message_log.borrow_mut().push(leave_msg.clone());
            if let Err(err) = self.out.broadcast(format!("{:?}", json!({
                "path": "/left",
                "content": leave_msg,
            }))) {
                error!("{:?}", err);
            }
        }
    }

    fn on_timeout(&mut self, tok: ws::util::Token) -> ws::Result<()> {
        match tok {
            SAVE => {
                let mut file = try!(OpenOptions::new().write(true).open(FILE));
                if let Err(err) = serde_json::to_writer_pretty::<_, Vec<Message>>(
                    &mut file,
                    self.message_log.borrow().as_ref())
                {
                   Ok(error!("{:?}", err))
                } else {
                    self.out.timeout(SAVE_TIME, SAVE)
                }
            }
            PING => {
                try!(self.out.ping(Vec::new()));
                self.out.timeout(PING_TIME, PING)
            }
            _ => unreachable!()
        }
    }
}

fn main () {

    // Setup logging
    env_logger::init().unwrap();

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(FILE)
        .expect("Unable to open message log.");

    let message_log = MessageLog::new(
        RefCell::new(
            serde_json::from_reader(
                &mut file,
            ).unwrap_or(
                Vec::with_capacity(10_000))));

    let users = Users::new(RefCell::new(HashSet::with_capacity(10_000)));

    if let Err(error) = ws::listen(ADDR, |out| {
        ChatHandler {
            out: out,
            nick: None,
            message_log: message_log.clone(),
            users: users.clone(),
        }

    }) {
        error!("Failed to create WebSocket due to {:?}", error);
    }
}
