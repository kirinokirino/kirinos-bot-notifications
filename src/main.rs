// note this uses `smol`. you can use `tokio` or `async_std` or `async_io` if you prefer.
// extensions to the Privmsg type
use twitchchat::PrivmsgExt as _;
use twitchchat::{
    messages::{Commands, Privmsg},
    runner::{AsyncRunner, NotifyHandle, Status},
    UserConfig,
};

#[macro_use]
extern crate log;
use simple_logger::SimpleLogger;
use std::fs::OpenOptions;
use std::io::prelude::*;

// this is a helper module to reduce code deduplication
mod include;
use crate::include::{channels_to_join, get_user_config};

use rand::prelude::*;
use std::collections::{HashMap, HashSet};
use std::process::Command;

fn main() -> anyhow::Result<()> {
    SimpleLogger::new().init().unwrap();

    // you'll need a user configuration
    let user_config = get_user_config()?;
    // and some channels to join
    let channels = channels_to_join()?;

    let start = std::time::Instant::now();
    let _result = Command::new("./queue").spawn();

    let mut bot = Bot::default()
        .with_action("!raid", move |_args: Args| {
            play_notification("/home/kirinokirino/audio/raid.ogg");
        })
        .with_action("!follow", |_args: Args| {
            play_notification("/home/kirinokirino/audio/follow.ogg");
        })
        .with_action("!host", |_args: Args| {
            play_notification("/home/kirinokirino/audio/host.ogg");
        })
        .with_action("!sr", |args: Args| {
            let message: Vec<&str> = args.msg.data().split(' ').skip(1).collect();
            //println!("{:#}", message.join(" "));
            song_request(message);
        })
        .with_action("!uptime", move |args: Args| {
            let output = format!("its been running for {:.2?}", start.elapsed());
            // We can send a message back (without quoting the sender) using a writer + our output message
            args.writer.say(args.msg, &output).unwrap();
        })
        .with_action("!quit", move |args: Args| {
            let output = format!("its been at least {:.0?}, time to rest! ", start.elapsed());
            // We can send a message back (without quoting the sender) using a writer + our output message
            args.writer.say(args.msg, &output).unwrap();
            // because we're using sync stuff, turn async into sync with smol!
            smol::block_on(async move {
                // calling this will cause read_message() to eventually return Status::Quit
                args.quit.notify().await
            });
        });
    /*
        .with_action("LUL", |args: Args| {
            let output = format!("LUL {}!", args.msg.name());
            // We can 'reply' to this message using a writer + our output message
            args.writer.reply(args.msg, &output).unwrap();
        })
    ;
    });
    */

    // run the bot in the executor
    smol::block_on(async move { bot.run(&user_config, &channels).await })
}

struct Args<'a, 'b: 'a> {
    msg: &'a Privmsg<'b>,
    writer: &'a mut twitchchat::Writer,
    quit: NotifyHandle,
}

trait Action: Send + Sync {
    fn handle(&mut self, args: Args<'_, '_>);
}

impl<F> Action for F
where
    F: Fn(Args<'_, '_>),
    F: Send + Sync,
{
    fn handle(&mut self, args: Args<'_, '_>) {
        (self)(args)
    }
}

#[derive(Default)]
struct Bot {
    commands: HashMap<String, Box<dyn Action>>,
    chatters: HashSet<String>,
    vip: HashSet<String>,
}

impl Bot {
    // add this action to the bot
    fn with_action(mut self, name: impl Into<String>, cmd: impl Action + 'static) -> Self {
        self.commands.insert(name.into(), Box::new(cmd));
        self
    }

    // run the bot until its done
    async fn run(&mut self, user_config: &UserConfig, channels: &[String]) -> anyhow::Result<()> {
        // this can fail if DNS resolution cannot happen
        let connector = twitchchat::connector::smol::Connector::twitch()?;

        let mut runner = AsyncRunner::connect(connector, user_config).await?;
        info!("connecting, we are: {}", runner.identity.username());

        self.chatters.insert("kirinokirino".to_string());
        self.chatters.insert("Wizebot".to_string());
        self.vip.insert("kirinokirino".to_string());
        self.vip.insert("lisadikaprio".to_string());
        self.vip.insert("wizebot".to_string());
        self.vip.insert("bacing".to_string());
        for channel in channels {
            warn!("joining: {}", channel);
            if let Err(err) = runner.join(channel).await {
                error!("error while joining '{}': {}", channel, err);
            }
        }

        // if you store this somewhere, you can quit the bot gracefully
        // let quit = runner.quit_handle();

        info!("starting main loop");
        self.main_loop(&mut runner).await
    }

    // the main loop of the bot
    async fn main_loop(&mut self, runner: &mut AsyncRunner) -> anyhow::Result<()> {
        // this is clonable, but we can just share it via &mut
        // this is rate-limited writer
        let mut writer = runner.writer();
        // this is clonable, but using it consumes it.
        // this is used to 'quit' the main loop
        let quit = runner.quit_handle();

        loop {
            // this drives the internal state of the crate
            match runner.next_message().await? {
                // if we get a Privmsg (you'll get an Commands enum for all messages received)
                Status::Message(Commands::Privmsg(pm)) => {
                    // see if its a action and do stuff with it
                    let (cmd, name) = Self::name_message(pm.data(), pm.name());

                    if self.vip.contains(name) {
                        if let Some(cmd) = cmd {
                            if let Some(action) = self.commands.get_mut(cmd) {
                                debug!("dispatching to: {}", cmd.escape_debug());

                                let args = Args {
                                    msg: &pm,
                                    writer: &mut writer,
                                    quit: quit.clone(),
                                };

                                action.handle(args);
                            }
                        }
                    }

                    self.new_chatter(name);
                }
                // stop if we're stopping
                Status::Quit | Status::Eof => break,
                //Status::Message(Commands::Whisper(notice)) => warn!("{:?}", notice),
                //Status::Message(Commands::Raw(notice)) => warn!("{:?}", notice),
                // ignore the rest
                Status::Message(..) => continue,
            }
        }

        info!("end of main loop");
        Ok(())
    }

    fn name_message<'a>(message: &'a str, name: &'a str) -> (Option<&'a str>, &'a str) {
        error!("{}: {}", name, message);

        let cmd = message.splitn(2, ' ').next();
        (cmd, name)
    }

    fn new_chatter(&mut self, chatter: &str) {
        if !self.chatters.contains(chatter) {
            self.chatters.insert(chatter.to_string());
            welcome_chatter();
        }
    }
}

fn play_notification(file: &str) {
    if let Err(error) = Command::new("mpv")
        .arg(file)
        .arg("--ao=jack")
        .arg("--jack-port=notification")
        .arg("--really-quiet")
        .spawn()
    {
        error!("Can't play notification! ERROR: {}", error);
    }
}

fn song_request(links: Vec<&str>) {
    let mut file = OpenOptions::new()
        .write(true)
        .append(true)
        .open("queue.txt")
        .unwrap();

    for link in links {
        if let Err(e) = writeln!(file, "{}", link) {
            eprintln!("Couldn't write to file: {}", e);
        }
    }
}

fn welcome_chatter() {
    let mut rng = rand::thread_rng();
    let random: f32 = rng.gen();
    let percent = (random * 100.0) as u8; // generates a float between 0 and 1
    match percent {
        0..=20 => play_notification("/home/kirinokirino/audio/welcome0.ogg"),
        21..=40 => play_notification("/home/kirinokirino/audio/welcome1.ogg"),
        41..=60 => play_notification("/home/kirinokirino/audio/welcome2.ogg"),
        61..=80 => play_notification("/home/kirinokirino/audio/welcome3.ogg"),
        _ => play_notification("/home/kirinokirino/audio/welcome4.ogg"),
    }
}
