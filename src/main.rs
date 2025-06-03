use std::{path::PathBuf, sync::Arc};

use clap::Parser;

mod dump_git;
mod git_parsing;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// The url of the exposed .git directory
    #[arg()]
    url: String,

    #[arg(short, long)]
    user_agent: Option<String>,
    /// The directory to download to
    #[arg(default_value = "git-dumped")]
    path: PathBuf,

    /// Sets the maximum of concurrent download tasks that can be running
    #[arg(short, long, default_value_t = 8)]
    tasks: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = Args::parse();

    if dbg!(!args.url.ends_with("/")) {
        args.url.push('/');
    }

    std::fs::create_dir_all(args.path.join(".git"))?;
    dump_git::download_all(Arc::new(args)).await;

    Ok(())
}
