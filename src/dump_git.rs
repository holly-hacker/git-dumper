use std::{collections::HashSet, path::PathBuf};

use regex::Regex;
use reqwest::StatusCode;
use tokio::sync::mpsc::{self, Sender};

use crate::git_parsing::{parse_hash, parse_head, parse_log, parse_object, GitObject};

lazy_static::lazy_static! {
    static ref REGEX_OBJECT_PATH: Regex = Regex::new(r"[\da-f]{2}/[\da-f]{38}").unwrap();
}

const START_FILES: &[&str] = &[
    "info/exclude",
    "logs/HEAD",
    "objects/info/packs", // TODO: this does not seem to be present anymore?
    "config",
    "COMMIT_EDITMSG",
    "description",
    "FETCH_HEAD",
    "HEAD",
    "index",
    "ORIG_HEAD",
    "packed-refs",
    "refs/remotes/origin/HEAD", // guessing remote names seems pointless, it's `origin` 99% of the time
];

// TODO: brute-force files based on known and unknown branch names

#[derive(Default)]
struct DownloadCache {
    /// The URL of the exposed .git directory
    base_url: String,
    /// The local path to download the git repo to
    base_path: PathBuf,
    cache: HashSet<String>,
}

impl DownloadCache {
    fn new(base_url: String, base_path: PathBuf) -> Self {
        Self {
            base_url,
            base_path,
            cache: HashSet::new(),
        }
    }

    /// Downloads a file if it hasn't been downloaded before and sends it to the given channel.
    fn download(&mut self, file_name: &str, sender: Sender<DownloadedFile>) {
        if self.cache.contains(file_name) {
            // println!("Skipping download of file {file_name} as it's already downloaded");
            return;
        }

        self.cache.insert(file_name.into());

        let url = format!("{}{file_name}", self.base_url);
        let file_name = file_name.into();
        tokio::spawn(async move {
            let got = reqwest::get(&url).await;
            match got {
                Ok(resp) => {
                    match resp.status() {
                        StatusCode::OK => {
                            let bytes = resp.bytes().await.unwrap(); // TODO: ugh, fix this unwrap
                            sender
                                .send(DownloadedFile {
                                    name: file_name,
                                    content: bytes.to_vec(),
                                    sender: sender.clone(),
                                })
                                .await
                                .unwrap();
                        }
                        StatusCode::NOT_FOUND => {
                            println!("Got 404 while trying to download {url}")
                        }
                        _ => {
                            println!(
                                "Error while trying to download {url}: status code is {}",
                                resp.status()
                            )
                        }
                    }
                }
                Err(e) => {
                    println!("Error while trying to download {url}: {e}");
                }
            }
        });
    }

    fn download_object(&mut self, object_hash: &str, sender: Sender<DownloadedFile>) {
        let hash_start = &object_hash[0..2];
        let hash_end = &object_hash[2..];
        let path = format!("objects/{hash_start}/{hash_end}");
        self.download(&path, sender)
    }
}

#[derive(Debug)]
struct DownloadedFile {
    pub name: String,
    pub content: Vec<u8>,
    pub sender: Sender<DownloadedFile>,
}

pub async fn download_all(base_url: String, base_path: PathBuf) {
    let mut cache = DownloadCache::new(base_url, base_path);

    // TODO: try out unbounded channel too
    // TODO: maybe just have a cli option that determines the limit of concurrent downloads instead?
    let (tx, mut rx) = mpsc::channel(32);

    for &file in START_FILES {
        let new_tx = tx.clone();
        cache.download(file, new_tx);
    }

    // drop the sender object so all senders can be out of scope by the end of the download
    drop(tx);

    // every time we downloaded a new file, see what other files we can derive from it
    while let Some(message) = rx.recv().await {
        // write this file to disk
        let path = cache.base_path.join(".git").join(&message.name);
        let path_parent = path
            .parent()
            .expect("There should be at least .git as parent");

        std::fs::create_dir_all(path_parent).unwrap_or_else(|e| {
            println!(
                "Error while trying to create directory {}: {:?}",
                path_parent.to_string_lossy(),
                e
            );
        });
        std::fs::write(path, &message.content).unwrap_or_else(|e| {
            println!(
                "Error while trying to write {} to disk: {:?}",
                message.name, e
            );
        });

        println!(
            "Downloaded '{}' ({} bytes)",
            message.name,
            message.content.len()
        );

        match message.name.as_str() {
            "HEAD" | "refs/remotes/origin/HEAD" => match parse_head(&message.content) {
                Ok(ref_path) => {
                    println!("\tFound ref path {ref_path}");
                    cache.download(ref_path, message.sender.clone());
                }
                Err(err) => println!("Failed to parse file {}: {:?}", message.name, err),
            },
            n if n.starts_with("refs/heads/") || n == "ORIG_HEAD" => {
                match parse_hash(&message.content) {
                    Ok(hash) => {
                        println!("\tFound object hash {hash}");
                        cache.download_object(hash, message.sender.clone());
                    }
                    Err(err) => println!("Failed to parse file {}: {:?}", message.name, err),
                }
            }
            // TODO: handle FETCH_HEAD, detect branches
            // TODO: handle config, detect branches
            n if n.starts_with("logs/") => match parse_log(&message.content) {
                Ok(hashes) => {
                    println!("\tFound log with {} hashes", hashes.len());
                    for hash in hashes {
                        cache.download_object(&hash, message.sender.clone());
                    }
                }
                Err(err) => println!("Failed to parse file {}: {:?}", message.name, err),
            },
            n if n.starts_with("objects/") && REGEX_OBJECT_PATH.is_match(n) => {
                match parse_object(&message.content) {
                    Ok(GitObject::Blob) => {
                        println!("\tFound blob object");
                    }
                    Ok(GitObject::Tree(hashes)) => {
                        println!("\tFound tree object with {} hashes", hashes.len());
                        for hash in hashes {
                            cache.download_object(&hash, message.sender.clone());
                        }
                    }
                    Ok(GitObject::Commit(hashes)) => {
                        println!("\tFound commit object with {} hashes", hashes.len());
                        for hash in hashes {
                            cache.download_object(&hash, message.sender.clone());
                        }
                    }
                    Err(err) => println!("Failed to parse file {}: {:?}", message.name, err),
                }
            }
            n => {
                println!("\tNot using file '{n}' for anything right now");
            }
        }
    }
}
