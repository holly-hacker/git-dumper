use std::path::PathBuf;

use clap::Parser;

mod dump_git;
mod git_parsing;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The url of the exposed .git directory
    #[arg()]
    url: String,

    /// The directory to download to
    #[arg(default_value = "git-dumped")]
    path: PathBuf,

    /// Sets the maximum of concurrent download tasks that can be running
    #[arg(short, long, default_value_t = 8)]
    tasks: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    // println!("URL: {url}");
    // println!("PATH: {path}");

    std::fs::create_dir_all(args.path.join(".git"))?;
    dump_git::download_all(args.url.clone(), args.path, args.tasks).await;

    Ok(())
}
