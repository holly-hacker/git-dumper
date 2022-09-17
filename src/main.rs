use std::path::PathBuf;

use clap::{command, Arg, Command};

mod dump_git;
mod git_parsing;

fn cli() -> Command<'static> {
    command!()
        .arg(
            Arg::new("URL")
                .required(true)
                .help("The url of the exposed .git directory"),
        )
        .arg(
            Arg::new("PATH")
                .required(false)
                .help("The directory to download to")
                .default_value("git-dumped"),
        )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = cli().get_matches();
    let url = matches.get_one::<String>("URL").unwrap();
    let path = matches.get_one::<String>("PATH").unwrap();

    // println!("URL: {url}");
    // println!("PATH: {path}");

    std::fs::create_dir_all(format!("{path}/.git/"))?;

    dump_git::download_all(url.clone(), PathBuf::from(path)).await;

    Ok(())
}
