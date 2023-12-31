mod config;

use anyhow::Ok;
use config::{Cmd, Config, RmanStdio};
use log::{Level, LevelFilter};
use std::{
    env,
    fs::{self, File},
    process::Stdio,
    sync::Arc,
};
use tokio::{process, sync::Mutex, task::JoinSet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .filter(None, LevelFilter::Info)
        .init();

    let pids: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let config = Config::init()?;
    let mut handles = JoinSet::new();

    let mut iter = config.cmds.iter().peekable();
    while let Some(command) = iter.next() {
        let is_last = iter.peek().is_none();
        let Cmd {
            name,
            cmd,
            args,
            stdin,
            stdout,
        } = command.clone();
        let pids = pids.clone();

        handles.spawn(async move {
            log::info!("rman running [{name}]: {cmd} {}", args.join(" "));
            let mut child = process::Command::new(cmd)
                .current_dir(env::current_dir().unwrap())
                .args(args.to_owned())
                .stdin(match stdin {
                    RmanStdio::Inherit => Stdio::inherit(),
                    RmanStdio::Null => Stdio::null(),
                    RmanStdio::File(path) => {
                        let file = File::open(path).unwrap();
                        Stdio::from(file)
                    }
                    _ => Stdio::inherit(),
                })
                .stdout(match stdout {
                    RmanStdio::Inherit => Stdio::inherit(),
                    RmanStdio::Null => Stdio::null(),
                    RmanStdio::File(path) => {
                        let file = File::open(path).unwrap();
                        Stdio::from(file)
                    }
                    _ => Stdio::inherit(),
                })
                .spawn()
                .expect("err: ");

            pids.lock().await.push(
                child
                    .id()
                    .unwrap_or_else(|| panic!("error getting pid for {name}")),
            );

            // write pids after spawning last command
            if is_last {
                let home = std::env::var("HOME").expect("$HOME env missing");
                let _ = fs::write(
                    format!("{home}/.rman.pids"),
                    format!("{:?}\n", pids.lock().await),
                );
            }

            child
                .wait()
                .await
                .expect("child process encountered an error");
        });
    }

    loop {
        tokio::select! {
        // exit if all commands complete
        _ = handles.join_next() => {
            if handles.is_empty() {
                break;
            }
        }
        _ = tokio::signal::ctrl_c() => {
            log::log!(Level::Info, "attempting graceful shutdown of {} processes", handles.len());
            handles.shutdown().await;
            break;
        }
        }
    }

    Ok(())
}
